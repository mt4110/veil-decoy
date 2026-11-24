import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomBytes, randomUUID } from "node:crypto";
import { once } from "node:events";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { get } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import { Miniflare, Response, convertV4MiniflareOptions } from "miniflare";
import { exitOf, sha256, startWarden, until } from "./warden.mjs";

const script = fileURLToPath(import.meta.url);
const root = resolve(dirname(script), "../..");
const edge = join(root, "crates/veil-decoy-edge");
const build = join(edge, "build");
const webhook = "https://hooks.slack.com/services/VEIL/LAB/LOCAL_ONLY";

if (process.argv[2] === "--probe") {
    const [, , , operation, input] = process.argv;
    const ready = once(process.stdin, "end");
    process.stdin.resume();
    await ready;
    if (operation === "read") {
        readFileSync(input, "utf8");
    } else {
        const url = new URL(input);
        assert.equal(url.protocol, "http:");
        assert.equal(url.hostname, "127.0.0.1");
        const request = get(url, { agent: false, family: 4, timeout: 5_000 });
        request.once("timeout", () => request.destroy(new Error("Probe timeout")));
        const [response] = await once(request, "response");
        response.resume();
        await once(response, "end");
        assert.equal(response.statusCode, 404);
    }
} else {
    await run();
}

async function run() {
    const output = process.env.VEIL_TRIAL_OUTPUT
        ? resolve(process.env.VEIL_TRIAL_OUTPUT)
        : mkdtempSync(join(tmpdir(), "veil-decoy-v2-"));
    if (process.env.VEIL_TRIAL_OUTPUT) mkdirSync(output, { mode: 0o700 });
    const trialId = randomUUID();
    const admin = randomBytes(32).toString("hex");
    const messages = [];
    let slackStatus = 200;
    const bindings = { VEIL_ADMIN_TOKEN: admin, SLACK_WEBHOOK_URL: webhook };
    const files = readdirSync(build, { recursive: true });
    const options = convertV4MiniflareOptions({
        host: "127.0.0.1",
        compatibilityDate: "2026-11-23",
        modulesRoot: root,
        modules: [
            { type: "ESModule", path: join(dirname(script), "fixture.js") },
            { type: "ESModule", path: join(edge, "worker.js") },
            ...files.filter((name) => name.endsWith(".js") || name.endsWith(".wasm"))
                .map((name) => ({
                    type: name.endsWith(".wasm") ? "CompiledWasm" : "ESModule",
                    path: join(build, name),
                })),
        ],
        bindings,
        d1Databases: ["VEIL_EVENTS"],
        kvNamespaces: ["VEIL_TOKENS"],
        ratelimits: {
            VEIL_ALERT_LIMIT: { namespace_id: "1001", simple: { limit: 3, period: 60 } },
            VEIL_REQUEST_LIMIT: { namespace_id: "1002", simple: { limit: 30, period: 60 } },
        },
        outboundService: async (request) => {
            assert.equal(request.url, webhook);
            assert.equal(request.method, "POST");
            const alert = await request.json();
            const eventId = /^Event: (\S+)$/m.exec(alert.text)?.[1];
            assert.ok(eventId);
            messages.push({ id: eventId, at_ms: Date.now(), status: slackStatus });
            return new Response("fixture", {
                status: slackStatus,
                headers: slackStatus === 429 ? { "Retry-After": "120" } : {},
            });
        },
    });
    const mf = new Miniflare(options);
    let observer;
    try {
        const base = await mf.ready;
        base.hostname = "127.0.0.1";
        const db = await mf.getD1Database("VEIL_EVENTS");
        const migration = join(root, "migrations/0001_decoy_events.sql");
        await db.batch(readFileSync(migration, "utf8").split(";").map((sql) => sql.trim())
            .filter(Boolean).map((sql) => db.prepare(sql)));
        const auth = { authorization: "Bearer " + admin, "content-type": "application/json" };
        const request = (path, init) => mf.dispatchFetch(new URL(path, base), init);
        const events = async (id) => {
            const result = await request("/events?token_id=" + id, { headers: auth });
            assert.equal(result.status, 200);
            return result.json();
        };
        const register = async (label) => {
            const id = randomUUID();
            const result = await request("/register", {
                method: "POST", headers: auth,
                body: JSON.stringify({ id, name: trialId + "/" + label }),
            });
            assert.equal(result.status, 201, await result.text());
            return id;
        };
        const retry = async () => {
            const result = await request("/_lab/retry", { method: "POST", headers: auth });
            assert.equal(result.status, 204, await result.text());
        };
        assert.equal((await request("/events?token_id=" + randomUUID())).status, 401);
        const valid = await register("registered");
        const unknown = randomUUID();
        const url = (id) => new URL("/ping/" + id, base).href;
        const targetFile = join(output, "synthetic-wiki.txt");
        writeFileSync(targetFile, url(valid) + "\n", { mode: 0o600 });

        if (process.env.VEIL_WARDEN_BINARY) {
            observer = await startWarden(
                process.env.VEIL_WARDEN_BINARY, process.env.VEIL_WARDEN_SHA256, output,
            );
        }
        const cases = [];
        for (const [name, operation, input, id, expectedTcp, expected] of [
            ["read_file", "read", targetFile, valid, 0, 0],
            ["registered_http", "get", url(valid), valid, 1, 1],
            ["unknown_http", "get", url(unknown), unknown, 1, 0],
        ]) {
            const startedAt = Date.now();
            const result = spawn(process.execPath, [script, "--probe", operation, input], {
                stdio: ["pipe", "inherit", "inherit"],
            });
            const done = exitOf(result);
            try {
                if (observer) observer.include(result.pid);
            } finally {
                result.stdin.end();
                assert.deepEqual(await done, { code: 0, signal: null });
            }
            let seen = await events(id);
            if (expected) {
                seen = await until(async () => {
                    const records = await events(id);
                    return records[0]?.state === "delivered" && records;
                }, "registered notification");
            }
            await delay(80);
            assert.equal(seen.length, expected);
            assert.equal(messages.length, name === "read_file" ? 0 : 1);
            const delivered = seen.flatMap((row) => messages.filter((item) => item.id === row.id));
            assert.equal(delivered.length, expected);
            cases.push({
                name, started_at_ms: startedAt, checked_at_ms: Date.now(), pid: result.pid,
                expected_tcp: expectedTcp, decoy_events: seen, local_notifications: delivered,
            });
        }

        const observing = observer;
        observer = undefined;
        const warden = observing ? await observing.finish(cases, Number(base.port)) : null;

        const retryToken = await register("retry");
        slackStatus = 429;
        assert.equal((await request("/ping/" + retryToken)).status, 404);
        const pending = await until(async () => {
            const [row] = await events(retryToken);
            return row?.last_error === "http_429" && row;
        }, "stored 429");
        assert.equal(pending.state, "pending");
        assert.equal(pending.attempts, 1);
        const stored = await db.prepare("SELECT payload, next_attempt_at FROM decoy_events WHERE id = ?")
            .bind(pending.id).first();
        assert.ok(stored.payload);
        assert.ok(stored.next_attempt_at - Date.now() >= 110_000);
        await retry();
        assert.equal(messages.filter((item) => item.id === pending.id).length, 1);

        // Advance only the synthetic row to its retry window without a two-minute sleep.
        await db.prepare("UPDATE decoy_events SET next_attempt_at = 0 WHERE id = ?").bind(pending.id).run();
        slackStatus = 200;
        await Promise.all([retry(), retry()]);
        const [delivered] = await events(retryToken);
        assert.equal(delivered.state, "delivered");
        assert.equal(delivered.attempts, 2);
        assert.equal(delivered.last_error, null);
        const attempts = messages.filter((item) => item.id === pending.id);
        assert.deepEqual(attempts.map((item) => item.status), [429, 200]);
        assert.equal((await db.prepare("SELECT payload FROM decoy_events WHERE id = ?")
            .bind(pending.id).first()).payload, null);

        slackStatus = 403;
        const terminalToken = await register("terminal");
        await request("/ping/" + terminalToken);
        const terminal = await until(async () => {
            const [row] = await events(terminalToken);
            return row?.state === "failed" && row;
        }, "terminal webhook rejection");
        assert.equal(terminal.attempts, 1);
        assert.equal(terminal.last_error, "http_403");

        const receipt = {
            trial_id: trialId, warden, cases, delivery: { pending, delivered, attempts, terminal },
            inputs_sha256: {
                cargo_lock: sha256(join(root, "Cargo.lock")),
                npm_lock: sha256(join(dirname(script), "package-lock.json")),
                worker: sha256(join(build, "veil_decoy_edge_bg.wasm")),
                migration: sha256(migration),
            },
        };
        const file = join(output, "receipt.json");
        writeFileSync(file, JSON.stringify(receipt, null, 2) + "\n", { flag: "wx", mode: 0o600 });
        console.log("V2 receipt: " + file);
    } finally {
        try {
            if (observer) await observer.stop();
        } finally {
            await mf.dispose();
        }
    }
}

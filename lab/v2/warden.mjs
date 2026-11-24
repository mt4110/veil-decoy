import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync, readdirSync, rmdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { setTimeout as delay } from "node:timers/promises";

export const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

export function exitOf(child) {
    return new Promise((resolve, reject) => {
        child.once("error", reject);
        child.once("close", (code, signal) => resolve({ code, signal }));
    });
}

export async function until(read, description) {
    for (let attempt = 0; attempt < 100; attempt++) {
        const result = await read();
        if (result) return result;
        await delay(100);
    }
    throw new Error("Timed out waiting for " + description);
}

export async function startWarden(binary, expectedHash, output) {
    assert.equal(process.platform, "linux", "Warden requires Linux");
    assert.equal(process.arch, "arm64", "Use the lab's ARM64 Linux VM");
    assert.deepEqual(readdirSync("/sys/class/net"), ["lo"], "Use a VM without a network interface");
    assert.equal(process.getuid(), 0, "The isolated VM needs privilege to attach the observer");
    const binaryHash = sha256(binary);
    assert.equal(binaryHash, expectedHash, "Warden binary SHA-256");
    const suffix = randomBytes(8).toString("hex");
    const scope = "/sys/fs/cgroup/warden-observe-" + suffix + ".scope";
    mkdirSync(scope);
    const child = spawn(binary, ["--scope", scope, "--seconds", "45"], {
        stdio: ["ignore", "pipe", "inherit"],
    });
    const done = exitOf(child);
    let exited = false;
    done.then(() => { exited = true; }, () => { exited = true; });
    const rows = [];
    const reader = createInterface({ input: child.stdout });
    reader.on("line", (line) => {
        appendFileSync(join(output, "warden.ndjson"), line + "\n");
        try {
            rows.push(JSON.parse(line));
        } catch {
            rows.push({ kind: "invalid_json" });
        }
    });

    const stop = async () => {
        try {
            if (!exited) child.kill("SIGTERM");
            assert.deepEqual(await done, { code: 0, signal: null });
        } finally {
            rmdirSync(scope);
        }
    };

    try {
        const ready = await until(() => {
            if (exited) throw new Error("Warden exited before readiness");
            return rows.find((row) => row.kind === "ready");
        }, "Warden readiness");
        assert.deepEqual(ready.hooks, ["connect4", "connect6"]);
        return {
            include(childPid) {
                writeFileSync(join(scope, "cgroup.procs"), childPid + "\n");
            },
            async finish(probes, port) {
                await stop();
                const events = rows.filter((row) => row.kind === "connection.attempt");
                const stats = rows.find((row) => row.kind === "stats" && row.final);
                assert.equal(events.length, 2);
                assert.equal(stats?.attempts, 2);
                assert.equal(stats?.decoded, 2);
                assert.equal(stats?.ring_dropped, 0);
                assert.equal(stats?.decode_errors, 0);
                assert.ok(!rows.some((row) => row.kind === "invalid_json"));
                for (const probe of probes) {
                    probe.warden = events.filter((event) => event.tgid === probe.pid);
                    assert.equal(probe.warden.length, probe.expected_tcp);
                    for (const event of probe.warden) {
                        assert.equal(event.destination, "127.0.0.1");
                        assert.equal(event.port, port);
                        assert.equal(event.cgroup_id, ready.cgroup_id);
                    }
                }
                return {
                    boot_id: readFileSync("/proc/sys/kernel/random/boot_id", "utf8").trim(),
                    loader_sha256: binaryHash,
                    final_stats: stats,
                };
            },
            stop,
        };
    } catch (error) {
        await stop();
        throw error;
    }
}

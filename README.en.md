# veil-decoy

A CLI and Cloudflare Worker for HTTP honeytokens. Opening a registered URL sends its placement label, source IP, country code, and User-Agent to Slack. Link previews and automated scanners can also trigger alerts. Check the placement and source when investigating an alert.

[日本語 README](README.md) · [MIT License](LICENSE)

## Setup

Use Rust 1.100.0, Node.js 26.13.1 LTS, and OSV-Scanner 2.7.1. With mise, review the repository configuration and run `mise trust` and `mise install`. Make Rustup, Wrangler, and OSV-Scanner available on your shell PATH.

```sh
git config --local core.hooksPath .githooks
cargo install --locked --path crates/veil-decoy-cli
cargo install --locked --version 0.2.130 wasm-bindgen-cli
```

Confirm that `wasm-bindgen --version` reports `0.2.130` before building the Worker.

The pre-commit hook validates the staged workspace with `cargo metadata --locked` and scans `Cargo.lock` and the lab's `package-lock.json`. A Cargo lockfile mismatch, a finding, or a scanner failure blocks the commit. After fixing dependencies, stage the manifests and updated lockfiles. To enforce this check on GitHub, require `Cargo dependency policy` on `main`.

## Deploy

```sh
wrangler login
wrangler kv namespace create VEIL_TOKENS
wrangler d1 create veil-decoy-events
```

Set the KV and D1 IDs in `wrangler.toml`. If the rate-limit `namespace_id` values are already in use in your Cloudflare account, choose unique values. Generate and securely store a random 64-character hexadecimal admin token, for example with `openssl rand -hex 32`.

```sh
wrangler d1 migrations apply veil-decoy-events --remote
wrangler deploy
wrangler secret put VEIL_ADMIN_TOKEN
wrangler secret put SLACK_WEBHOOK_URL
```

Set `VEIL_ADMIN_TOKEN` in the CLI environment. Store `SLACK_WEBHOOK_URL` in the Worker. Upgrading an existing Worker also requires the admin token for registration, listing, and revocation.

## Register, verify, and revoke a honeytoken

```sh
export VEIL_EDGE_URL=https://veil-decoy-edge.example.com

trap_url=$(veil-decoy generate http --name "wiki-prod-access")
curl --silent --output /dev/null --write-out "%{http_code}\n" "$trap_url"
veil-decoy list
veil-decoy events <token-id>
# Use the ID from the list when taking down the token.
veil-decoy revoke <token-id>
```

The CLI prints the URL after successful registration. The URL returns HTTP `404` even when it triggers an alert. Open a test URL and verify Slack receives the alert before placing a honeytoken. See the [wiki example](examples/wiki-setup.md) for a sample placement.

Unknown and revoked IDs do not trigger alerts. KV propagation can delay registration and revocation at other locations. Approximate limits apply per Cloudflare location. Up to 30 requests per minute per source proceed to lookup and up to three per registered token proceed to delivery. Suppressed notifications after a match appear as `rate_limited` events.

The Worker records matches in D1 before notifying Slack. It retries timeouts, HTTP 429, and 5xx responses up to six total attempts. `events` returns up to 50 recent records per token with the event ID, detection time as Unix milliseconds, delivery state, attempt count, and last error. Event records expire after seven days. D1 keeps the alert body while delivery is pending. An interruption after Slack accepts the message can cause another delivery with the same event ID.

## V2 isolated trial

The [lab procedure](lab/v2/README.md) runs the Worker with a local mock and optionally correlates its output with Warden in an ARM64 Linux VM.

## Offline credential decoys

`generate aws|redis|api|postgres|slack --name <label>` produces synthetic credentials locally. These formats provide offline decoys without alert delivery. Redis and PostgreSQL placeholders use the reserved `.invalid` domain.

Existing `/ping/<token_id>` links still work, and previous KV records can be listed or revoked. New records store the placement label, type, timestamp, and ID.

## Related projects

[veil-warden](https://github.com/mt4110/veil-warden) monitors outbound traffic with eBPF.

# veil-decoy

**Honeytoken Generator & Tracker** — Intrusion detection through fake credentials

[![CI](https://github.com/mt4110/veil-decoy/actions/workflows/ci.yml/badge.svg)](https://github.com/mt4110/veil-decoy/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

> 🇯🇵 [日本語 README](README.md)

---

## Concept

Instead of raising walls, **catch attackers the moment they step inside**.

`veil-decoy` generates realistic-looking fake API keys and connection strings (honeytokens).  
The moment an attacker attempts to use one, it captures their IP, country, and User-Agent  
and sends an instant Slack alert.

```
🚨 [Veil Decoy Alert] Honeytoken triggered!
• Trap   : production-db-trap
• Token  : a3f2...
• IP     : 198.51.100.x (RU)
• UA     : python-requests/2.31.0
⚡ Initiate incident response immediately.
```

---

## Why it works

| Traditional detection | veil-decoy |
|---|---|
| Predict attack vs. normal → high false-positive rate | Token triggered = 100% confirmed breach |
| Hard to pinpoint the leak source | Instantly know which repo / file was compromised |
| Attacker moves with confidence | "Is this a trap?" paranoia slows them down |

---

## Architecture

```
┌─────────────────────┐       POST /register        ┌─────────────────────────┐
│   veil-decoy (CLI)  │ ─────────────────────────→  │  veil-decoy-edge        │
│   Rust binary       │                              │  Cloudflare Workers     │
└─────────────────────┘                              └────────────┬────────────┘
         │                                                        │
    generates                                             attacker hits
  honeytoken string                                     GET /ping/<token_id>
         │                                                        │
    embedded in                                          ┌────────▼────────┐
  .env / wiki / code                                     │  Slack Webhook  │
                                                         └─────────────────┘
```

---

## Quickstart

### 1. Install

```bash
git clone https://github.com/mt4110/veil-decoy.git
cd veil-decoy
cargo build --release -p veil-decoy-cli
```

### 2. Generate a trap

```bash
veil-decoy generate aws --name "production-s3-trap"
veil-decoy generate redis --name "cache-server-trap"
veil-decoy generate api --name "internal-api-trap"
```

### 3. Deploy the Cloudflare Workers edge

```bash
npx wrangler login
npx wrangler kv:namespace create VEIL_TOKENS   # paste the ID into wrangler.toml
npx wrangler secret put SLACK_WEBHOOK_URL
npx wrangler deploy
```

### 4. Register and monitor

```bash
export VEIL_EDGE_URL=https://veil-decoy-edge.<your-account>.workers.dev

veil-decoy generate aws --name "project-x-trap" --edge-url $VEIL_EDGE_URL
# => ✅ Registered with edge server — monitoring active.
```

Embed the output string in `.env.backup`, an internal wiki, or a stale config file.  
When an attacker uses it, you'll know immediately.

---

## Supported token types

| kind | Impersonates | Example |
|---|---|---|
| `aws` | AWS IAM key pair | `AKIAIOSFODNN7EXAMPLE` |
| `redis` | Redis connection URL | `redis://admin:xxx@trap…:6379` |
| `api` | Generic Bearer token | `veil_xxxx…` |
| `postgres` | PostgreSQL DSN | `postgresql://postgres:xxx@trap…` |
| `slack` | Slack Bot Token | `xoxb-xxx-xxx-xxx` |

---

## The veil ecosystem

| Repo | Role |
|---|---|
| **veil-decoy** (this repo) | 🪤 Lure and expose intruders |
| [veil-warden](https://github.com/mt4110/veil-warden) | 🛡️ eBPF-based outbound traffic monitor |

---

## License

[MIT](LICENSE)

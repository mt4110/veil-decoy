# veil-decoy

**偽装機密情報ジェネレーター＆トラッカー** — ハニートークン (Honeytoken) による侵入検知

[![CI](https://github.com/mt4110/veil-decoy/actions/workflows/ci.yml/badge.svg)](https://github.com/mt4110/veil-decoy/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

> 🇬🇧 [English README](README.en.md)

---

## コンセプト

防御の壁を高くするのではなく、**「侵入されたことに1秒で気づく」** ための罠を CLI から一瞬で構築します。

`veil-decoy` は本物そっくりの偽 API キー・接続文字列（ハニートークン）を生成し、  
それが悪用された瞬間に攻撃者の IP・国・User-Agent を収集して Slack へ即時通知します。

```
🚨 [Veil Decoy Alert] Honeytoken triggered!
• Trap   : production-db-trap
• Token  : a3f2...
• IP     : 198.51.100.x (RU)
• UA     : python-requests/2.31.0
⚡ Initiate incident response immediately.
```

---

## なぜ有効か

| 従来の検知 | veil-decoy |
|---|---|
| 通常通信と攻撃を「予測」する → 誤検知が多い | 罠に触れた = 100% 確定的な侵害証拠 |
| 侵害ルートの特定に時間がかかる | どのリポジトリ/ファイルから漏れたか即特定 |
| 攻撃者は自信を持って動く | 「どれが罠か？」という疑心暗鬼でスピード低下 |

---

## アーキテクチャ

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

## クイックスタート

### 1. インストール

```bash
git clone https://github.com/mt4110/veil-decoy.git
cd veil-decoy
cargo build --release -p veil-decoy-cli
# または
cargo install --path crates/veil-decoy-cli
```

### 2. 罠を生成する

```bash
# AWS アクセスキーに偽装した罠
veil-decoy generate aws --name "production-s3-trap"

# Redis 接続文字列に偽装した罠
veil-decoy generate redis --name "cache-server-trap"

# その他: api / postgres / slack
veil-decoy generate api --name "internal-api-trap"
```

### 3. Cloudflare Workers エッジをデプロイする

```bash
# Cloudflare アカウントでログイン
npx wrangler login

# KV ネームスペースを作成（表示された ID を wrangler.toml に記入）
npx wrangler kv:namespace create VEIL_TOKENS

# Slack Webhook URL を Secret として登録
npx wrangler secret put SLACK_WEBHOOK_URL

# デプロイ
npx wrangler deploy
```

### 4. エッジと連携して罠を仕掛ける

```bash
export VEIL_EDGE_URL=https://veil-decoy-edge.<your-account>.workers.dev

veil-decoy generate aws --name "project-x-trap" --edge-url $VEIL_EDGE_URL
# => ✅ Registered with edge server — monitoring active.
```

生成された文字列を `.env.backup` や社内 Wiki に埋め込んでおきます。  
攻撃者がそれを使おうとした瞬間、Slack に通知が届きます。

---

## 仕掛け方のサンプル

[`examples/`](examples/) ディレクトリに以下のテンプレートがあります。

- [`.env.production.trap`](examples/.env.production.trap) — `.env` ファイルへの埋め込み例
- [`wiki-setup.md`](examples/wiki-setup.md) — 社内 Wiki への埋め込み例

---

## 対応するハニートークンの種類

| kind | 偽装対象 | 例 |
|---|---|---|
| `aws` | AWS IAM アクセスキーペア | `AKIAIOSFODNN7EXAMPLE` |
| `redis` | Redis 接続文字列 | `redis://admin:xxx@trap…:6379` |
| `api` | 汎用 Bearer トークン | `veil_xxxx…` |
| `postgres` | PostgreSQL DSN | `postgresql://postgres:xxx@trap…` |
| `slack` | Slack Bot Token | `xoxb-xxx-xxx-xxx` |

---

## veil エコシステム

| リポジトリ | 役割 |
|---|---|
| **veil-decoy** (本リポジトリ) | 🪤 侵入した敵を炙り出す罠 |
| [veil-warden](https://github.com/mt4110/veil-warden) | 🛡️ eBPF で外への通信を監視・遮断する盾 |

---

## ライセンス

[MIT](LICENSE)

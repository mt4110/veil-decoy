# veil-decoy

HTTP ハニートークンで、不意のアクセスを検知する CLI と Cloudflare Worker です。登録した URL が開かれると、配置名・アクセス元 IP・国コード・User-Agent を Slack に通知します。リンクプレビューや自動スキャンでも通知されるため、受信時は配置先やアクセス元を確認してください。

[English README](README.en.md) · [MIT License](LICENSE)

## セットアップ

Rust 1.100.0、Node.js 26.13.1 LTS、OSV-Scanner 2.7.1 を使用します。mise を使う場合は、リポジトリの設定を確認してから `mise trust` と `mise install` を実行します。シェルの PATH に Rustup、Wrangler、OSV-Scanner を通してください。

```sh
git config --local core.hooksPath .githooks
cargo install --locked --path crates/veil-decoy-cli
cargo install --locked --version 0.2.130 wasm-bindgen-cli
```

Worker のビルド時に、PATH 上の `wasm-bindgen --version` が `0.2.130` であることを確認してください。

pre-commit は Git にステージされたワークスペースを `cargo metadata --locked` で確認し、`Cargo.lock` と隔離試験の `package-lock.json` を OSV-Scanner で検査します。Cargo のロックファイルとの不整合、OSV の指摘、スキャナーの実行失敗のいずれでもコミットは止まります。依存を修正したらマニフェストと更新したロックファイルをステージし直してください。GitHubで強制する場合は `Cargo dependency policy` を `main` の必須チェックに設定します。

## デプロイ

```sh
wrangler login
wrangler kv namespace create VEIL_TOKENS
wrangler d1 create veil-decoy-events
```

作成した KV と D1 の ID を `wrangler.toml` に設定します。同じ Cloudflare アカウントで既に使われている場合は、レート制限の `namespace_id` を重複しない値に変更します。64桁のランダムな16進数を `openssl rand -hex 32` などで生成し、管理トークンとして安全に保管してください。

```sh
wrangler d1 migrations apply veil-decoy-events --remote
wrangler deploy
wrangler secret put VEIL_ADMIN_TOKEN
wrangler secret put SLACK_WEBHOOK_URL
```

`VEIL_ADMIN_TOKEN` を CLI の環境変数に設定します。`SLACK_WEBHOOK_URL` は Worker に登録します。既存 Worker の更新時も、登録・一覧・失効に管理トークンが必要になります。

## 登録・確認・失効

```sh
export VEIL_EDGE_URL=https://veil-decoy-edge.example.com

trap_url=$(veil-decoy generate http --name "wiki-prod-access")
curl --silent --output /dev/null --write-out "%{http_code}\n" "$trap_url"
veil-decoy list
# 一覧で得たIDの直近50件を確認します
veil-decoy events <token-id>
# 撤去時は一覧で得た ID を使います
veil-decoy revoke <token-id>
```

登録が成功した場合に、CLI は URL を標準出力に返します。有効な URL でも HTTP 応答は `404` です。設置前にテスト用の URL を開き、Slack の受信を確認してください。配置例は [Wiki 用テンプレート](examples/wiki-setup.md) にあります。

未登録・失効済みの ID へのアクセスは通知対象外です。Workers KV の伝播により、別拠点への登録・失効の反映に時間がかかります。レート制限は Cloudflare 拠点ごとの目安です。アクセス元ごとに毎分30件まで照合し、登録済みURLごとに毎分3件を超えて通知を抑えた場合は `rate_limited` として記録します。

Worker は照合結果を D1 に保存し、Slack に通知します。429、5xx、タイムアウトなどは分単位の間隔と `Retry-After` を使って最大6回まで試みます。`events` ではイベントID、検知時刻（Unixミリ秒）、配送状態、試行回数、直近の失敗理由を確認できます。イベントの保存期間は7日です。通知先に送る本文は再送に必要な間だけD1に置きます。受信側が成功を返した直後に状態保存が中断すると同じイベントが再配送されることがあるため、通知内のイベントIDで照合してください。

## V2の隔離試験

ローカルの模擬通知先でファイル読み取り、登録済みURL、未登録URLを比較し、別の試行で通知の再送を確認できます。Wardenと突き合わせる場合はARM64 Linux隔離VMで同じ試験を実行します。 [再現手順](lab/v2/README.md)

## 資格情報に似せたデコイ

`generate aws|redis|api|postgres|slack --name <label>` は、偽の資格情報をローカルで生成します。これらの形式には通知機能がありません。Redis と PostgreSQL は予約済みの `.invalid` ドメインを使います。

既存の `/ping/<token_id>` リンクは引き続き使えます。旧バージョンの KV に残るレコードは一覧・失効の対象です。新規登録は配置ラベル・種別・登録時刻・ID を保存します。

## 関連プロジェクト

[veil-warden](https://github.com/mt4110/veil-warden) は eBPF で外向き通信を監視します。

# 社内 Wiki への配置例

自分が管理する Wiki に置く HTTP ハニートークンの例です。ページやファイルを特定できるよう、配置ごとにラベルを変えます。

```sh
veil-decoy generate http --name "wiki/legacy-staging/access"
```

ページ内のリンクを、CLI が出力した URL に差し替えます。

```markdown
旧ステージングの管理画面は [こちら](https://veil-decoy-edge.example.com/ping/REPLACE_WITH_TOKEN_ID)。
```

Wiki がリンクをプレビューする場合、ページを保存した時点で通知が来ることがあります。配置前のテストには別のトークンを使い、撤去時は `veil-decoy list` から対象 ID を確認して `veil-decoy revoke <token-id>` を実行します。

# 旧検証環境のセットアップメモ（2024-03 廃止予定）

> **注意**: 以下の接続情報は旧ステージング環境のものです。現在は使用されていません。

## Redis（旧キャッシュ層）

```
redis://admin:REPLACE_WITH_GENERATED@trap.veil-edge.workers.dev:6379/0
```

## 社内 Kubernetes クラスター

```yaml
# kubeconfig excerpt
clusters:
  - cluster:
      server: https://trap.veil-edge.workers.dev/k8s
    name: internal-staging
```

## AWS（旧バッチ処理用）

```bash
export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
export AWS_DEFAULT_REGION=ap-northeast-1
```

---

*このページは社内Wikiの片隅に置いておく「仕掛け見本」です。
実際のトークンは `veil-decoy generate` で生成して差し替えてください。*

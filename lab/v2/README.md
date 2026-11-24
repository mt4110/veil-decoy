# V2 の隔離試験

サンプルのファイル読み取り、登録済みの HTTP URL、未登録の HTTP URLを順に試す。Worker の検知結果とローカルの模擬通知をイベントIDで突き合わせる。Linux隔離VMでWardenも実行する場合、試行用の子プロセスだけを専用cgroupに入れ、PID・宛先・ポートと最終統計で接続試行を照合する。

| 試行 | WardenのTCP接続試行 | Decoyの検知 | 模擬通知 |
| --- | ---: | ---: | ---: |
| ファイルを読む | 0 | 0 | 0 |
| 登録済みURLに接続する | 1 | 1 | 1 |
| 未登録URLに接続する | 1 | 0 | 0 |

模擬通知先は初回のHTTP 429に対して `Retry-After: 120` を返す。試験は再送前の D1 の記録を確認し、合成データの再送予定時刻だけを進める。実際の Worker のスケジュール処理を二つ並行で呼び、同じイベントIDの再送が1回で終わることと、403が終端エラーになることを確認する。

## ローカル試験

リポジトリルートで、Rust 1.100.0、wasm-bindgen-cli 0.2.130、Node.js 26.13.1 をPATHに用意して実行する。

~~~sh
npm ci --prefix lab/v2
./lab/v2/build-worker.sh
npm test --prefix lab/v2
~~~

レシートの保存先は標準出力に表示する。保存先を指定するなら、未作成のディレクトリを `VEIL_TRIAL_OUTPUT` に設定する。ローカル版では `warden: null` を記録する。試験中の通知はすべてMiniflareの模擬受信先で処理される。

## Wardenを含めたVM試験

Apple Silicon Mac上で、V2の照合済みDebian ARM64ベースイメージ、抽出済みの `linux-arm64` と `initrd`、QEMU/HVF、観測専用WardenのARM64バイナリとそのSHA-256を使う。Node.js v26.13.1 の Linux ARM64 tarball は、Node.js公式リリースの `SHASUMS256.txt` で確認する。準備時に npm 依存関係を解決し、ゲストにはバイナリと固定済みの依存だけを渡す。

~~~sh
./lab/v2/build-worker.sh
mkdir -p "$linux_deps"
cp lab/v2/package.json lab/v2/package-lock.json "$linux_deps/"
npm ci --prefix "$linux_deps" --os=linux --cpu=arm64 --ignore-scripts
./lab/v2/qemu.sh "$qemu" "$debian_image" "$boot_dir" "$warden" "$warden_sha256" \
  "$linux_node_tar" "$linux_deps/node_modules" "$result_dir"
~~~

各引数は絶対パスで、`result_dir` には親が存在する未作成のディレクトリを指定する。VMには仮想NICとホストディレクトリ共有を設定せず、ベースディスクは試験専用オーバーレイを経由して起動する。Wardenのフック取り付けに必要な権限はゲスト内で使う。終了時に専用cgroupを取り除き、ホストに `receipt.json`、`warden.ndjson`、`serial.log` とSHA-256を保存する。異常終了時はシリアルログから停止段階を確認できる。

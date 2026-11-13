# DRCS収集・解析用ソース

製品の文字判定は `src/drcs.rs` と `src/drcs/mappings.rs` の64bitハッシュ・Unicode対応表 で完結します。
このディレクトリーは収集・解析のための開発用ソースです。通常のDebug/Releaseビルドには組み込みません。

|ソース|用途|有効化方法|
|---|---|---|
|`collector.rs`|未登録の受信ビットマップをBMP・JSONへ保存|`drcs-collect` feature / `build.ps1 -CollectDrcs`|
|`caption_probe.rs`|指定した録画TSから字幕JSONを抽出|`caption-tools` feature / `--bin caption_probe`|

```powershell
# 通常のランタイムで動く32bit収集DLL（Debugランタイム不要）
./build.ps1 -Architecture x86 -CollectDrcs

# Debug収集DLL（Debugランタイムが必要）
./build.ps1 -Architecture x86 -Debug -CollectDrcs

# 録画の字幕解析のみ
cargo run --release --features caption-tools --bin caption_probe -- sample.ts captions.jsonl 1040

# 録画の字幕解析と未対応画像の収集
cargo run --release --features caption-tools,drcs-collect --bin caption_probe -- sample.ts captions.jsonl 1040
```

収集先はホスト実行ファイル直下の `drcs-unmapped/` です。caption_probeの場合はその実行ファイルの隣になります。
製品に収集済みBMP・JSONを探索・読み戻しする処理はありません。対応文字の選定とハッシュ・Unicode対応表への追加は別の作業です。収集IDは製品の照合キーと同じFNV-1aです。

## コミット時の区分

- 製品：`src/` の判定・変換ソース、ハッシュ対応表、関連テスト。
- 開発ツール：このディレクトリーのソースと、Cargo・ビルドスクリプトのfeature設定。製品変更と区別してレビューできます。
- ローカル資料：開発用の画像・JSON・録画。`drcs-unmapped/` はGit管理から除外しています。

開発ツールのソースは画像データと異なり、featureを有効にしたビルドを再現するためGit管理対象です。ソースを `.gitignore` で隠す運用はしません。
収集ツールを別コミットにする場合は、`src/lib.rs` の条件付きモジュール宣言と `src/drcs.rs` の条件付き収集処理も、feature設定と一緒に含めてください。

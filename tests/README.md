# テスト構成

- `unit/`：DRCS、通信、番組情報、配信、DRCS収集の単体テスト。
- 直下の `.rs`：公開APIのRust結合テスト。
- 直下の `.cpp`：C++互換APIとDLLの結合テスト。

`unit/` は非公開関数も検証するため、対象モジュールの `#[cfg(test)]` と `#[path]` 宣言から読み込みます。製品ビルドには含まれません。テストの実装はこのディレクトリーに集約しています。

```powershell
cargo test
cargo test --all-features
```

`--all-features` ではDRCS収集のテストも実行します。テスト用ビットマップは人工的に生成し、収集資料や外部の置換JSONには依存しません。C++とDLLも含む検証はルートの `test.ps1` で実行します。

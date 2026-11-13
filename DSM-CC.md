# DSM-CC / BML連携

本DLLはDSM-CCセクション、コンポーネント情報、PCRをWebSocketで通知します。モジュールの復元やBMLの再生は受信側の処理です。この文書のサーバー・ブラウザー機能は、対応するtvrockgo実装との連携仕様を説明しています。サーバーとフロントエンドのソース・実行ファイルは本リポジトリに含まれません。

## 転送の有効化とロゴ受領通知

BML関連の転送は既定で無効です。解析・転送コードは保持しており、録画・視聴アプリの起動引数に`/bml`を追加すると、以下で説明するBMLセクション・PMT・PCRを転送します。字幕とEPG、CDTロゴはこの指定に依存しません。DSM-CCでは、BML用として識別されたコンポーネントを除き、ロゴを含み得る受信機向け・エンジニアリング用コンポーネントのDII/DDBを転送します。

対応サーバーがロゴモジュール全体の保存に成功すると、`DSMModuleReceived`コマンドで受領を返します。messageは`sid`・`onid`・`tsid`・`pid`・`downloadId`・`moduleId`・`version`を持つJSONです。DLLは一致するDDBを再送しません。DIIは引き続き送り、新しいバージョンも送ります。受領状態はDLLクライアントのメモリーに保持し、プロセス再起動時は再取得します。保存失敗・受領通知の欠落時は再送を続けます。旧サーバーとの組み合わせでは受領による抑止は働きません。

## DLLの送信処理

`src/dsm.rs` と `src/engine.rs` がPAT/PMTからDSM-CC PIDを特定し、TSの分割・連続性・CRCを確認して `DSM:` JSONを送信します。stream_type 0x0B/0x0D、DII/DDB、イベント記述子、選択サービスのPCRに対応します。

完全なDSM-CCセクションは16進文字列として送ります。

```text
DSM:{"Sid":101,"Tsid":1,"Onid":4,"Pid":288,"Components":[{"Pid":288,"ComponentTag":64,"DataComponentId":7,"Info":"3340f880"}],"Data":"3b..."}
```

`Kind:"pmt"` はコンポーネント一覧、`PCRBase` / `PCRExtension` は再生時刻です。DDBのsection_numberやcurrent_next_indicatorを通常のPSI制約として扱いません。

## 受信側の処理

対応するtvrockgo実装では、次の処理を行います。

|場所|処理|
|---|---|
|`tvrock/deviceserver.go`|DSM通知の受信。復元状態をデバイス接続ごとに保持し、選局時に破棄|
|`dsm.go`, `dsm_manifest.go`, `dsm_events.go`|複数セクションDII、順不同DDB、バージョン更新、zlib圧縮、ARIB MIME multipart、放送イベント・PCR|
|`bml.go`|BMLから静的HTMLへの変換。UTF-8・Shift_JIS・EUC-JPを処理し、静的HTMLのスクリプトとイベント属性を除去|
|`app/src/components/BMLWindow.vue`|BML再生、静的HTML表示、再読込、数字・矢印・決定・戻る・四色・dキー|
|`DeviceController.vue`, `App.vue`|字幕の下への表示、WebSocket受信、選局・切断・PMT変更時の消去|

サーバーからブラウザーへは `bml:` を送ります。全ブロックが揃ったモジュールだけを通知し、受信中は最新スナップショットを15秒ごとにも送ります。ブラウザーはモジュール版を確認し、繰り返し受信による再起動を抑制します。

### 局ロゴ

`LOGO-00`～`LOGO-05`および`CS_LOGO-00`～`CS_LOGO-05`のモジュールから、対象サービス一覧と画像を解析します。受信側の `receiveCDT` / `saveCDT` を利用し、実行ファイル基準の `static/cdt/<内部Sid>_<LogoType>.png` に保存します。BS/CS混在モジュールはONIDで振り分けます。CDTの通知仕様は [CDT.md](CDT.md) を参照してください。

## 導入と確認

1. DSM-CC対応のサーバーとフロントエンドを用意します。サーバーを交換する場合は既存プロセスを終了し、二重起動を避けてください。
2. 視聴アプリ等のホストを終了し、ホストと同じビット数の `TvRockDTV.dll` に置き換えます。DLLの入手・ビルド手順は [README.md](README.md) を参照してください。
3. BMLを利用する場合はホストの起動引数に`/bml`を指定し、フロントエンドを配置してブラウザーを再読み込みします。対応するデータ放送を受信するとBMLウィンドウを表示します。
4. 音声は、ブラウザーの自動再生制限により最初のリモコン操作で有効になります。

DLL側のテストは本リポジトリのルートで実行します。

```powershell
cargo generate-lockfile
cargo test --locked
./test.ps1 -Architecture x64
./test.ps1 -Architecture x86
```

サーバー側のビルド・テストは対応するtvrockgoリポジトリで実行してください。実設定を読み込むTestMainがDBやスケジューラーを起動する構成では、テスト用の設定・データを分離する必要があります。フロントエンドのビルドが出力先 `static` を消去する場合は、受信済みの `static/cdt` 等を保持できる別の出力先を使用してください。

## 対応範囲

受信側の復元処理は国内ARIBデータカルーセルのMIMEモジュールを対象とし、汎用BIOPオブジェクトカルーセルのファイルシステム復元には対応しません。モジュールは圧縮前後それぞれ2 MiBまで、デバイスごとの宣言サイズ合計は16 MiBまでに制限します。

動的BMLはweb-bmlのBML用ECMAScriptインタープリターで再生し、放送スクリプトをアプリ本体のJavaScriptとして直接実行しません。双方向通信、受信機固有ID、選局APIは接続していません。対応範囲は受信側とライブラリの実装に依存し、全API・全放送局の互換性を保証しません。静的HTML表示は確認用で、通常はBML再生を使用します。

DLLの検証には合成TSとローカルWebSocketを使用します。関連する受信側実装では、合成データによる自動起動、決定キーでのスクリプト実行、HTML表示切替を確認しています。実放送TSでの局別動作は未検証です。

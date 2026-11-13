# TvRockDTV Rust移植版

Windows用のTvRockDTV互換DLLです。TSの解析、番組情報、字幕、ARIB文字変換、局ロゴ・DSM-CCの通知、WebSocket通信、SRT・UDPプロトコルによる調整・配信をRustで実装しています。

既存ホストとの互換性を保つため、`CreateTvRockDTV` が返すMSVC C++仮想クラスとホストコールバックの寿命管理は `compat/TvRockDTV.cpp` に実装しています。公開クラスの仮想メソッド順とfactory名を維持し、Rustのtrait objectをC++の仮想クラスとして直接公開しません。

## 主な機能

- 分割TS入力、PAT/PMT、SDT/NIT、EITを解析し、サービス情報と現在／次番組を通知します。PSI/SIのCRC32、連続性、重複・欠落を検査します。
- 番組をONID/TSID/SID/EIDで区別し、拡張説明を含む変更を通知します。EITのイベント共有（group_type=1、参照先1件）は`CommonSid`・`CommonEid`でEIT/CPF通知へ渡し、短形式の番組説明がない共有イベントも通知します。共有指定のないイベントは両項目をnullで送ります。子自身のSID・EID・時刻を保持し、共有先の番組内容の解決は受信側で行います。日時はJSTから64ビットUnix時刻へ変換します。
- ARIB字幕の文字、表示時刻、横書き座標、文字サイズ、色・背景色を解析します。確認済みのDRCS字形はUnicodeへ変換します。
- CDTロゴを抽出し、SDTのロゴ伝送記述子と対応付けます。DSM-CCのロゴ候補も転送します。BML用のセクション・コンポーネント情報・PCRの転送は`/bml`指定時に有効です。詳細は [CDT.md](CDT.md) と [DSM-CC.md](DSM-CC.md) を参照してください。
- WebSocket接続、Bearer認証、再接続、状態の再送、命令キューをDLL内で処理します。

## DLLの入手と導入

タグのビルドが成功すると、GitHub Releasesに `tvrockdtv.zip` が公開されます。対象バージョンのAssetsから取得し、ホストアプリと同じビット数のDLLを選んでください。

```text
tvrockdtv/
  x64/TvRockDTV.dll
  x86/TvRockDTV.dll
```

ホストアプリを終了してから、従来の `TvRockDTV.dll` と置き換えます。実行には対応するビット数のVisual C++ランタイムが必要です。

## ビルド

必要な環境はWindows、Rust MSVC toolchain（Rust 1.98以降）、Visual Studio C++ Build ToolsとWindows SDKです。通常のビルド／テストにPythonは不要です。初回はCargo依存クレートのダウンロードが必要です。TLSにはWindowsのSchannelを使用します。

リポジトリのルートで実行します。

```powershell
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc
cargo generate-lockfile
./build.ps1 -Architecture x64
./build.ps1 -Architecture x86
```

`Cargo.lock` はGit管理対象外です。新規チェックアウトでは `cargo generate-lockfile` で作成してください。`build.ps1` は `--locked` を使用し、生成済みの依存バージョンでビルドします。

|構成|出力先|
|---|---|
|x64 Release|`target/x86_64-pc-windows-msvc/release/TvRockDTV.dll`|
|x86 Release|`target/i686-pc-windows-msvc/release/TvRockDTV.dll`|
|Debug|同じターゲットの `debug/TvRockDTV.dll`|

`-Debug` でデバッグ版を作成します。Debug版にはMSVCデバッグランタイムが必要です。`cargo build` 単独では互換DLLを作成できないため、DLLのビルドには `build.ps1` を使用してください。

PowerShellの実行ポリシーで拒否される場合は、次のように実行します。

```powershell
powershell -ExecutionPolicy RemoteSigned -File ./build.ps1 -Architecture x64
```

## 通信

ホストの起動引数は `/host サーバー /pid 登録コード /d BonDriver.dll` です。`/pid` はサーバーのRegistryCodeで、録画命令のdeviceidとは別です。`/host` はホスト:ポートまたはhttp/https/ws/wssのURLを受け付け、接続先パスを `/dev` とします。既存のJSON／通知形式とBearer認証ヘッダーを使用します。

- 初期化時にCH2を送信し、Readyを最大20秒待ちます。Ready前のLoopBackにも応答します。
- LoopBackの `LTC:UNIXタイムスタンプ` には `LTC:UNIXタイムスタンプ,バージョン` で応答します。バージョンはビルド時のHEADに付いたGitタグ名です。タグなし、未コミットの変更あり、またはGit情報を取得できない場合は `dev` になります。
- WebSocket接続時に `User-Agent: TvRockDTV/バージョン` を送信します。バージョンはLoopBack応答と同じ値です。
- `/parent 親の登録コード` を追加すると兄弟モードになり、`User-Agent: TvRockDTV-Sib/バージョン` と `X-TvRockDTV-Parent: 親の登録コード` を送信します。`Authorization: Bearer 自分の登録コード` は引き続き `/pid` の値です。例: `/host localhost:8080 /pid 自分の登録コード /parent 親の登録コード`。`/parent` を指定したのに値がない、空、またはヘッダに使用できない値の場合は初期化に失敗します。親との関連付け処理はtvrockgo側で実装してください。
- 兄弟モードでは `.ch2` の読み込み・送信、TS由来の情報、状態通知などのアプリケーションデータの送信を行いません。受信コマンドは `PowerOff`（`TVROCK_DTV_POWER_OFF`）と `Capture`（`TVROCK_DTV_SCREEN_CAPTURE`）のみ実行し、`Ready`、`LoopBack`、録画・選局・設定命令は無視します。WebSocket接続成立時に初期化を完了し、接続維持用のPing/Pongと再接続は継続します。
- 接続処理は専用スレッドで実行します。録画ファイル名は命令ごとに所有し、イベント種別とチャンネル／SIDを別フィールドで扱います。
- CH2、DIF、STA、STC、CPF、SDTは未接続時も保持します。SDTはセクション単位で再送し、EITは接続世代が変わった後のTS処理で再送します。送信キューへの登録失敗時は次回へ持ち越します。
- 送信待ちは4096件／8 MiB、状態キャッシュは4 MiB、受信命令は256件、WebSocketメッセージは1 MiBが上限です。頻繁な状態通知は最新値にまとめます。上限超過時は非キャッシュ通知が失われる場合があります。受信命令が満杯の場合は再接続します。
- 接続・送信タイムアウトは5秒、再接続間隔は1秒、Pingは30秒、受信停止の判定は90秒です。PowerOff後は再接続しません。
- Disposeはワーカー終了を待ちます。コールバック自身から解放した場合は、ワーカー終了までDLLを保持します。
- 診断は `OutputDebugStringW` に出力し、認証コードや録画命令本文を記録しません。

## 字幕とDRCS

`CAP` 通知は `Timing:1`、`DelayMs`（PES PTSからの表示オフセット、ミリ秒）、`Layout`（放送ピクセル座標、字送り、文字寸法）を含みます。PTSは33ビット・90 kHzです。TIMEによる表示・消去時刻と区間ごとの書式を保持し、横書きのTIME、APS/ACPS、SWF/SDF/SDP、SSM/SHS/SVS、文字サイズ・色・背景色を処理します。対象サービスを指定した場合は他サービスの字幕を除外し、第一言語を表示します。

DRCSは字幕本文のデータユニット（0x30/0x31）を解析し、寸法・ビット深度・全ビットマップが登録データと一致する場合だけUnicodeへ変換します。現在の登録対象は36×36または18×36・2bppで、漢字、括弧、矢印、機器・音の記号などを含みます。対応一覧と代替字形の説明は [src/drcs/README.md](src/drcs/README.md) にまとめています。

外字コードの固定置換や画像の類似判定は行いません。未登録字形への再定義では以前の対応を破棄します。未対応文字は「□」となり、保存済みの「□」を後から復元することはできません。Unicode記号の表示には受信側の対応フォントが必要です。

### 未対応DRCSの収集

収集機能は `-CollectDrcs` を指定したビルドだけに含まれます。通常のRelease／Debugビルドでは収集しません。

```powershell
./build.ps1 -Architecture x86 -CollectDrcs
./build.ps1 -Architecture x64 -CollectDrcs
```

収集版DLLは `target/<target>/release/drcs-collector/TvRockDTV.dll` に出力されます。通常のVisual C++ランタイムで動作します。`-Debug` を併用した場合は `debug/drcs-collector/` に出力され、MSVCデバッグランタイムが必要です。

保存先はホスト実行ファイルのフォルダー直下の `drcs-unmapped/` です。初めて受信した未対応画像をBMPとJSONで保存します。JSONには元ビットマップ、幅・高さ、ビット深度、DRCSコード、フォントIDを含み、対応表の作成には両方を使用します。画像情報のハッシュをファイル名に使用し、同じ字形は重複保存しません。

収集は字幕本文のビットマップ方式（mode 0/1）が対象です。保存先に書き込み権限が必要で、保存失敗はデバッグ出力に記録します。権限を修正した後はホストを再起動してください。詳細は [tools/drcs/README.md](tools/drcs/README.md) を参照してください。

### 録画TSの解析

```powershell
cargo run --release --features caption-tools --bin caption_probe -- sample.ts captions.jsonl 1040
```

最後の引数はサービスIDで、省略時は全サービスを出力します。サーバーへ接続せず、字幕通知をJSON Lines形式で保存します。`caption-tools,drcs-collect` を指定すると未対応DRCSも収集します。

## テスト

```powershell
cargo generate-lockfile
./test.ps1 -Architecture x64
./test.ps1 -Architecture x86
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

RustのTS・番組情報・字幕・DRCS・CDT・DSM-CC・通信の回帰テスト、ARIBランダム入力、TS/EITランダム入力、C++互換クラスとの結合テスト、実DLLとローカルWebSocketサーバーの通信テストを実行します。DLLテストは連続録画命令、日本語ファイル名、チャンネル変更、再初期化、コールバック内の解放とアンロードを検証し、録画ファイルは作成しません。

## CIと配布物

GitHub Actions（`.github/workflows/ci.yml`）は全ブランチのpushで書式・Clippyチェックとx64/x86のテストを実行します。タグのpushではチェックとテストを再実行せず、x64/x86の配布物をビルドしてGitHub Releasesへ公開します。タグはブランチpushでテスト済みのコミットに付けてください。Pull Requestを契機とした実行は行いません。

GitLab CI（`.gitlab-ci.yml`）はタグのpushでのみ実行します。

```powershell
git tag v0.1.0
git push origin v0.1.0
```

GitHub ActionsのテストジョブはRust stable、Clippy、rustfmt、x64/x86のMSVCターゲットを準備します。タグのビルドジョブはRust stableと両MSVCターゲットを準備し、独立して実行します。`Cargo.lock` は各ジョブで生成するため、ブランチのテスト時とタグのビルド時で依存バージョンが異なる場合があります。GitLab CIでは従来どおりタグpush時にチェックとテストを実行し、生成した `Cargo.lock` をビルドジョブへ渡します。

GitHub ActionsではCargoの並列ビルドを2ジョブに制限し、dev/testプロファイルのデバッグ情報を無効にします。これらはCI限定の設定で、ローカルのデバッグビルド設定は変更しません。Rust依存のビルド結果と `native/.deps/` のソースをキャッシュし、Clippyはビルドスクリプトの出力を含む詳細ログを表示します。各ジョブの制限時間は60分です。ログが止まった場合は、ジョブの終了状態と詳細ログを確認してください。最後の `Compiling` 行だけではメモリ不足とは判断できません。

ネイティブ依存の取得は180秒、各アーカイブの展開は180秒、CMakeの設定は300秒、ビルドは900秒、インストールは120秒を上限とします。展開とCMakeは開始・完了・30秒ごとの待機ログを表示し、上限を超えると失敗します。tar.bz2の展開にはCMake内蔵のアーカイブ機能を使用し、ZIPは別のWindows PowerShellプロセスで展開します。標準入力は閉じて実行します。

`build-artifact.ps1` は両アーキテクチャのDLLを `tvrockdtv/x64/` と `tvrockdtv/x86/` に集めます。生成先はGit管理対象外で、ローカルでも同じスクリプトを使用できます。

GitHubは `GITHUB_TOKEN` とビルドジョブの `contents: write` 権限でReleaseを作成し、ZIPを添付します。既存Releaseでは同名ZIPを更新します。GitHub Actionsのartifactからも取得できます。

GitLabは `publish-gitlab-release.ps1` でZIPをGeneric Package Registryへ保存し、Releaseにリンクを登録します。`CI_JOB_TOKEN` を使用するため追加トークンは不要です。Package Registryを有効にしてください。非公開プロジェクトのダウンロードにはアクセス権が必要です。パッケージのバージョンにはジョブIDを使用し、古いパッケージは自動削除しません。

GitLab CIはWindows Runnerの `saas-windows-medium-amd64` を指定しています。独自Runnerではタグを変更し、Windows、PowerShell、Visual Studio C++ Build ToolsとWindows SDKを用意してください。

## ソース構成

|場所|内容|
|---|---|
|`src/ts.rs`, `src/engine.rs`|TS再構成、PAT/PMT、解析処理の統合|
|`src/si.rs`, `src/events.rs`|SDT/NIT、EIT、番組情報・リレー通知|
|`src/caption.rs`, `src/caption_layout.rs`|字幕PES、時刻・装飾・座標、CAP通知|
|`src/arib.rs`, `src/tables.rs`, `src/drcs.rs`|ARIB文字変換、外字・マクロ、DRCS照合|
|`src/drcs/`|DLLへ組み込む字形データと対応一覧|
|`src/cdt.rs`, `src/dsm.rs`|CDTロゴ、DSM-CC・PCRの通知|
|`src/dtvclient.rs`, `src/dtvclient_ffi.rs`|WebSocket通信と内部FFI境界|
|`src/lib.rs`, `src/ffi.rs`, `src/windows.rs`|モジュール構成、FFI、CP932変換|
|`src/time.rs`, `src/json.rs`|JST日時変換、JSONエスケープ|
|`compat/`|MSVC C++ ABIとホストコールバック|
|`tools/drcs/`|未対応DRCS収集、録画TS解析|
|`tools/protocol_harness.rs`|DLL通信テスト用サーバー|
|`tools/import_tables.py`|元のARIB文字対応表からのテーブル生成|
|`tests/`|Rust回帰テスト、C++結合・DLLロードテスト|
|`build.ps1`, `build-artifact.ps1`, `test.ps1`|ビルド、配布物作成、テスト|

## 対応範囲と制約

- MSVC x86/x64 ABIと合成TSでの検証を対象とします。録画TSで字幕の背景、本文・ルビサイズ、遅延消去を確認していますが、UDP配信は実チューナーとTvRockGO・TVTestで確認しています。長時間録画は未検証です。
- 受信インスタンスは1プロセスにつき1つです。シリーズ・イベントグループの記述子は内部Eventに生データで保持し、旧内部ヘッダーの構造体・グローバル変数は公開APIに含めません。
- 字幕の縦書き、DRCSビットマップ描画、管理データによる高度なレイアウト、管理データだけで定義されたDRCS、分割データグループの結合、字幕CRC16検証は未対応です。分割グループは部分表示せず無視します。
- 受信側の字幕表示には `Timing`、`DelayMs`、`Layout` の処理が必要です。関連する受信側実装の固定遅延は1秒で、外部動画プレイヤーとの完全同期は保証しません。既存字幕DBにはLayout／DelayMsを保存しないため、これらはライブ通知が対象です。受信側のソースは本リポジトリに含みません。
- WSSはOSの証明書検証を使用します。実TLSサーバーへの接続は未検証です。名前解決やホストコールバックが戻らない場合、Disposeは完了を待つ場合があります。
- FFI境界でRust panicを捕捉します。ホストは元APIと同様に有効なポインターとバッファを渡す必要があります。
- 64ビット時刻を送信しますが、2038年以降の扱いは受信側の実装にも依存します。

## UDPリモート配信

RockRequest_V101にStreamStart(address,port)、StreamStop()、GetStreamStatus(status)を追加しています。V100のABIは維持します。兄弟TVTestはセッション専用の/pidと/parentで接続し、録画側とは独立した準備・受信状態を報告します。TvRockGOとexe版Rockbarも対応版へ更新してください。設定手順はTvRockGOのdocs/remote-viewing.mdを参照してください。

RockRequest_V102のStreamStartEx(address,port,flags)で送信内容を指定できます。flagsは現在サービス=1、字幕=2、データ放送=4、ワンセグ=8です。ワンセグは現在サービス指定より優先し、V100/V101の仮想関数配置は維持します。V101ホストでは既定のflags=7のみ使用できます。

RockRequest_V103のStreamStartSession(session,address,port,flags)、StreamStopSession(session)、GetStreamSessionStatus(session,status)で視聴PCごとの独立した送信を制御します。各セッションは状態照会で更新するリースを持ち、1つの終了・切断は他の送信や録画を停止しません。V100/V101/V102の仮想関数配置は維持します。

## SRTリモート配信

対応版TvRockGO/RockbarからSRTを選択すると、RustのストリームワーカーがBonDriver_SRTのlistenerへcaller接続します。既存UDP API・ABIは変更していません。内部制御の `StreamStartSRT` は `session`、IPv4の `address`、`port`、`flags`、`leaseSeconds`、`latencyMs`（0〜60000）と `passphrase`（空または10〜79文字の印字可能ASCII）を受け付けます。状態には `srtSupported` と `transport` を追加し、秘密値は含めません。停止・状態照会・サービス抽出・リース切れはUDPと共通です。

SRT live/messageモードで最大1,316バイト（TS×7）ずつ送ります。接続と送信は非同期ソケットで行い、録画コールバックは有界キューへコピーするだけです。切断すると送信ワーカーを終了し、TvRockGOが視聴リースに従って再開します。UDP用の平準化要求はSRTには適用しません。暗号化はAES-256で、受信側INIと同じパスフレーズを指定します。TS配信の暗号化と制御接続のHTTPS/WSSは別々に設定します。

Haivision SRT 1.5.4とMbed TLS 3.6.5を静的リンクします。追加のSRT用DLLは不要です。ビルドにはPowerShell 7、MSVC x86/x64とWindows SDK、Windows版CMake/Ninjaが必要です。CMake/NinjaはVisual StudioのC++ CMake toolsから優先検出します。`cargo build/test/clippy` でも `build.rs` が依存を自動構築します。初回はネットワークを使い、SHA-256を検証したソースを `native/.deps/` に保存します。以降はキャッシュを使用します。オフライン環境では事前に `native/dependencies.json` の2つのアーカイブを同フォルダへ用意してください。SRT送信の対象はWindows MSVCです。

`build.ps1` の出力と `build-artifact.ps1` の配布フォルダには依存ライセンスと `THIRD_PARTY.md` を含めます。DLLを配布するときは同梱してください。

実BonDriverとの任意の統合テストは次のとおりです。DLLと公開ヘッダーのパスを指定し、ビット数を一致させます。

```powershell
./test-srt.ps1 -BonDriverDLL '<BonDriver_SRT.dllのパス>' -BonDriverInclude '<BonDriver_SRTのincludeフォルダ>'
```

専用フォルダとポート33230で、平文/AES-256のTSバイト一致、キー未設定・不一致拒否、再接続、分割入力、カウンター、接続待ち中の停止を確認します。利用者のINIは変更しません。実TVTestでの映像・音声、別PC間通信、パケット損失下の復旧は別途確認してください。

複数のSRT配信セッションはDLL内でランタイムを共有し、最後のセッションを終了するまで保持します。1つの視聴先の停止や接続失敗によって、他の視聴先のSRTソケットを閉じません。多重受信対応BonDriver_SRTとの `test-srt.ps1` では、2ポートへの同時配信と片方の停止後の送信継続も検証します。

## DLLの配信モード

UDP送信・サービス抽出・バッファ・送信状態はRustの`stream`モジュールで処理します。C++部分は既存APIとの互換アダプターです。TSTask／RecTestとTvRockOnTVTestは、配信TSを解析用キューとは別にDLLへ渡します。TVTest本体の変更は不要です。

V100のvtableを維持した追加エクスポートを`GetProcAddress`で取得します。

| エクスポート | 用途 |
| --- | --- |
| `TvRockDTVSetStreamMode(receiver, mode)` | 初期化前に配信モードを設定。従来ホストの送信中は切り替えできません。0は無効、1は`ProcessTS`から入力、2は専用入力。既定は0。受信インスタンスの`/parent`指定では無効化されます。 |
| `TvRockDTVStreamTS(receiver, data, size)` | モード2の配信専用TS入力。呼び出し中に必要なデータをコピーし、送信は別スレッドで行います。 |
| `TvRockDTVStreamReset(receiver)` | 選局・ドライバー切り替え時に保留データを破棄。次の`OnStreamChange`でサービスを確定するまで送信を保留します。 |
| `TvRockDTVStreamStart(receiver, address, port, flags, session)` | 配信開始。flagsは現在サービス1、字幕2、データ放送4、ワンセグ8。sessionに空文字列またはNULLを指定すると既定セッション。 |
| `TvRockDTVStreamStop(receiver, session)` | 対象セッションを停止。 |
| `TvRockDTVGetStreamStatus(receiver, status, session)` | 対象セッションの送信先・送信量・選択サービス・エラーを取得。statusのSizeを設定して呼び出します。 |

複数配信ではセッションごとに64文字の小文字16進IDを使用します。TvRockGOからのリモート視聴では既存プロトコルのままDLL内の送信を制御し、切断時のリース、古い停止要求の分離、複数PCの個別停止を維持します。直接APIで開始した配信は呼び出し側で終了し、`Dispose`も全送信を終了します。

RecTestはモードに`STREAM_MODE_RECORDER_PORT_LOCK`を併用して、本体の標準UDP用ポート予約を共有します。配信処理を更新するときは配信元のDLLを更新してください。移行時は配信元アプリまたはプラグインも対応版が必要です。

配信モードでは、ホストのRockRequest_V101〜V103の送信コールバックを使用せずRust側へ処理します。配信モードを有効にしない従来ホストでは、既存コールバックへの転送を維持します。


受信側TvRockOnTVTestは任意エクスポート`TvRockDTVReceiverQuality(receiver, drops)`へTVTestの累積DropPacketCountを報告します。DLLは受信セッションの準備状態、サービス、選局世代、品質情報の鮮度とともに親TvRockGOへ通知します。親が継続的なドロップを検出した場合、既存リースのあるセッションへの`StreamPace`要求でRust送信を小さいUDPと平準化へ切り替えます。通常方式は既定のままです。公開StreamStatusと既存vtableのABIは変更せず、送信報告の`pacedSupported`と`paced`で対応状況・実際のモードを確認できます。

## Playback接続

通常のリモート視聴（`/parent`付き接続）は従来どおり`RCV:`以外を送信しません。Playback対応のTvRockGOから`PlaybackControl`を受けると、対応するTvRockOnTVTestへ一時停止／リセットを依頼します。処理完了後に`PBA:`で世代番号を返し、解析した字幕を通常の`CAP:`から`PBC:`（世代番号＋字幕本文）へ変換して送ります。EPG・局情報・放送時刻などを通常デバイスへ送る経路には戻しません。再接続時はこのモードを解除し、サーバーからの再指定を待ちます。

再生・ファイル位置・SRT送出の管理はTvRockGO、TVTest本体の操作はTvRockOnTVTestが担当します。実際の映像停止とシーク後の復帰はTVTest・BonDriver_SRTとの実機確認が必要です。

### Playback制御の能力通知

`TvRockDTVPlaybackCapabilities(receiver,supported,ready)`は追加のオプションexportです。
TvRockOnTVTestがグラフ停止機能への対応と初期化状態を登録します。
Playback受信側の`RCV:`は`controlVersion:2`、`graphPauseSupported`、`graphPauseReady`を追加します。
`pause-direct`と`resume-direct`は既存のChannelChange拡張でTvRockOnTVTestへ渡し、`PBA:`に世代・操作名・成功可否を返します。
失敗応答では字幕送信を再開しません。従来のreset／pauseと世代だけのPBA応答は互換動作を維持します。
新方式の選択、失敗時の切り替え、SRT接続とファイル位置の管理はTvRockGOが担当します。

### Playbackの黒画面待受

受信状態の`controlVersion:3`と`idleSupported`で、TvRockOnTVTestの黒画面待受への対応を通知します。
`PlaybackControl`の`idle`を予約コールバックのservice=4へ渡し、成功・失敗を明示して応答します。
待受中は字幕転送を抑止し、次の`reset`成功後に再開します。

### EPG送信の優先度

現在・次番組のCPF通知は従来どおり即時送信します。EITは直近30分の情報を100ms間隔で優先確認し、それより先の情報は2秒間隔・最大64番組（通常256KiB以内）でまとめて送信します。送信キューが受け付けなかった番組は再試行対象に残します。プロトコルとC++ ABIは変更していません。

### BML転送とDSM-CCロゴ

BMLの解析・転送コードは保持していますが、通常起動時はBML用DSM-CC・PCR・PMT通知を転送しません。録画・視聴アプリの起動引数に`/bml`を付けた場合だけ従来のBML転送を有効にします。字幕・EPG・CDTロゴ・受信機向けDSM-CCロゴとエンジニアリングロゴの候補は、このフラグとは独立しています。

対応サーバーはDSM-CCのロゴを保存した後だけ`DSMModuleReceived`を返します。DLLは局・ネットワーク・TS・PID・ダウンロード・モジュール・バージョンの一致するDDBの再送を止めます。DIIは止めず、新しいバージョンや別のモジュールは送ります。受領情報はDLLクライアントのメモリー内に保持し、プロセス再起動では再取得します。旧サーバーから受領通知が来なければ、従来どおりロゴ候補を送ります。

## ライセンス

本プロジェクト固有のコードのライセンスは [MIT License](LICENSE) です。依存ライブラリのライセンス・著作権表示・SRTソースの入手先は [THIRD_PARTY.md](THIRD_PARTY.md) と [licenses/](licenses/) に記載しています。DLLの再配布時は `LICENSE`、`THIRD_PARTY.md`、`licenses/` 全体を同梱してください。`build.ps1` と `build-artifact.ps1` はこれらを配布先にコピーします。依存更新後の表記は `./tools/write-license-notices.ps1` で再生成できます。

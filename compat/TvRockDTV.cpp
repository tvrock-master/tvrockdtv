// MSVC C++ virtual-class ABI, host callback lifetime adapter; networking is in
// src/dtvclient.rs. TS, SI, EIT, caption and ARIB processing are implemented in
// src/*.rs.
//
#include "TvRockDTV.h"
#include "rust_core.h"
#include "stdafx.h"
#include "tvrock.h"
#include <algorithm>
#include <atomic>
#include <climits>
#include <fstream>
#include <locale.h>
#include <memory>
#include <map>
#include <mutex>
#include <new>
#include <process.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shlwapi.h>
#include <stdio.h>
#include <stdlib.h>
#include <string>
#include <time.h>
#include <vector>
#include <windows.h>

#define STATION_MAX (1024)

#define STATION_NAME_MAX 256
#ifdef _USRDLL
#define _DLL_EXPORT extern "C" __declspec(dllexport)
#else
#define _DLL_EXPORT
#endif

#pragma comment(lib, "shlwapi.lib")

using namespace TvRock;

namespace TvRock {

static std::recursive_mutex streamMutex;
static std::shared_ptr<void> activeClient;
static std::shared_ptr<void> CurrentClient() {
  std::lock_guard<std::recursive_mutex> guard(streamMutex);
  return activeClient;
}

static bool RustSend(const char *text) {
  const auto c = CurrentClient();
  return c && dtv_send(c.get(), text);
}

static bool RustValid(unsigned short sid) {
  const auto c = CurrentClient();
  return c && dtv_valid(c.get(), sid);
}

static uint32_t TunerType() {
  const auto c = CurrentClient();
  return c ? dtv_tuner(c.get()) : 255;
}

void SendDeviceServer(const wchar_t *format, ...) {
  const auto c = CurrentClient();
  if (!c || !format)
    return;
  va_list args;
  va_start(args, format);
  const int n = _vscwprintf(format, args);
  va_end(args);
  if (n < 0 || n > 1048576)
    return;
  std::wstring text(static_cast<size_t>(n) + 1, L'\0');
  va_start(args, format);
  const int written = vswprintf_s(&text[0], text.size(), format, args);
  va_end(args);
  if (written >= 0)
    dtv_send_wide(c.get(), text.data(), static_cast<size_t>(written));
}

void SendDeviceServer2(const wchar_t *text) {
  const auto c = CurrentClient();
  if (c && text)
    dtv_send_wide(c.get(), text, wcslen(text));
}

#define PROP_TVROCK_DEVICE_ID L"TvRock Device ID"

#ifdef NDEBUG
std::atomic<BOOL> flagVerbose = FALSE;
#else
std::atomic<BOOL> flagVerbose = TRUE;
#endif
TvRockDTV *myPtr = NULL;

static unsigned __stdcall RemoteFunc(LPVOID vdParam);

static std::wstring ReadDriverCh(std::istream &input) {
  char bom[3] = {};
  input.read(bom, sizeof(bom));
  const auto count = input.gcount();
  const bool utf16le = count >= 2 &&
                       static_cast<unsigned char>(bom[0]) == 0xff &&
                       static_cast<unsigned char>(bom[1]) == 0xfe;
  const bool utf8 = count == 3 && memcmp(bom, "\xef\xbb\xbf", 3) == 0;
  input.clear();
  input.seekg(utf16le ? 2 : utf8 ? 3 : 0, std::ios::beg);
  const UINT codePage = utf8 ? CP_UTF8 : 932;
  std::wstring message;
  for (int n = 0; n < STATION_MAX; ++n) {
    std::wstring wide;
    if (utf16le) {
      // UTF-16のコードユニットを完全な形で読み取る必要がある。
      // 0x0aというバイト値が日本語文字の一部である可能性もあるため
      // バイト単位で処理を行うgetlineはここでは使用できない。
      char low, high;
      while (input.get(low)) {
        if (!input.get(high))
          return {};
        const wchar_t ch = static_cast<unsigned char>(low) |
                           (static_cast<unsigned char>(high) << 8);
        if (ch == L'\n')
          break;
        wide += ch;
      }
      if (input.eof() && wide.empty())
        break;
    } else {
      std::string line;
      if (!std::getline(input, line))
        break;
      if (line.size() > INT_MAX)
        return {};
      if (!line.empty()) {
        const int length = MultiByteToWideChar(
            codePage, 0, line.data(), static_cast<int>(line.size()), NULL, 0);
        if (!length)
          return {};
        wide.resize(length);
        if (!MultiByteToWideChar(codePage, 0, line.data(),
                                 static_cast<int>(line.size()), &wide[0], length))
          return {};
      }
    }
    if (!wide.empty() && wide.back() == L'\r')
      wide.pop_back();
    wide += L'\n';
    message += message.empty() ? L"CH2:" : L"|";
    message += wide;
  }
  return message;
}

static void LoadDriverCh(const wchar_t *dname) {
  if (!dname || !*dname) {
    SendDeviceServer(L"CH2:file not found");
    return;
  }
  std::wstring path(dname);
  const size_t slash = path.find_last_of(L"\\/");
  const size_t dot = path.find_last_of(L'.');
  if (dot != std::wstring::npos && (slash == std::wstring::npos || dot > slash))
    path.resize(dot);
  path += L".ch2";
  std::ifstream input(path.c_str(), std::ios::binary);
  const std::wstring message = input ? ReadDriverCh(input) : std::wstring();
  SendDeviceServer2(message.empty() ? L"CH2:file not found" : message.c_str());
}

static void CopyPropertyText(WCHAR *destination, size_t capacity,
                             const WCHAR *source) {
  if (destination && capacity)
    wcsncpy_s(destination, capacity, source ? source : L"", _TRUNCATE);
}

class myStationInfo : public StationInfo {

public:
  WCHAR name[STATION_NAME_MAX];
  DWORD channel;
  DWORD service;

public:
  myStationInfo() : channel(0), service(0) { memset(name, 0, sizeof(name)); }
  // 局名
  void GetName(LPWSTR lpPath, SIZE_T MaxSize) {
    CopyPropertyText(lpPath, MaxSize, name);
  }
  void SetName(LPCWSTR lpPath) {
    CopyPropertyText(name, STATION_NAME_MAX, lpPath);
  }
  // チャンネル
  DWORD GetChannel() const { return channel; }
  void SetChannel(const DWORD ch) { channel = ch; }
  // サービス
  DWORD GetServiceID() const { return service; }
  void SetServiceID(const DWORD sv) { service = sv; }
};

class myRockProperty : public RockProperty {
  int devNo;

  WCHAR appPath[MAX_PATH];
  WCHAR appName[MAX_PATH];
  WCHAR appName2[MAX_PATH];

  WCHAR appOpt[MAX_PATH];
  WCHAR appOpt2[MAX_PATH];

  WCHAR filePath[MAX_PATH];
  WCHAR fileExt[MAX_PATH];

  DWORD staSmartCard;
  float statSignal;
  DWORD staError;
  DWORD staScrambling;
  DWORD staBitrate;

  myStationInfo stations[STATION_MAX];

public:
  myRockProperty(const int dno)
      : devNo(dno), staSmartCard(0), statSignal(0.0f), staError(0),
        staScrambling(0), staBitrate(0) {
    memset(appPath, 0, sizeof(appPath));
    memset(appName, 0, sizeof(appName));
    memset(appName2, 0, sizeof(appName2));
    memset(appOpt, 0, sizeof(appOpt));
    memset(appOpt2, 0, sizeof(appOpt2));
    memset(filePath, 0, sizeof(filePath));
    memset(fileExt, 0, sizeof(fileExt));
    CopyPropertyText(fileExt, _countof(fileExt), TEXT(".ts"));
  }

  void Notify() {} // The original Go Notify did not send a protocol message.

  // 地上波
  bool UseTS() const { return (TunerType() == TVROCK_TUNER_TYPE_TS); }

  // BS
  bool UseBS() const { return (TunerType() == TVROCK_TUNER_TYPE_BSCS); }

  // CS
  bool UseCS() const { return (TunerType() == TVROCK_TUNER_TYPE_BSCS); }

  // アプリケーションフォルダ
  void GetAppPath(LPWSTR lpPath, SIZE_T MAX_SIZE) {
    CopyPropertyText(lpPath, MAX_SIZE, appPath);
  }
  void SetAppPath(LPCWSTR lpPath) {
    CopyPropertyText(appPath, MAX_PATH, lpPath);
  }

  // 実行アプリ名（視聴有り）
  void GetAppName(LPWSTR lpPath, SIZE_T MAX_SIZE) {
    CopyPropertyText(lpPath, MAX_SIZE, appName);
  }
  void SetAppName(LPCWSTR lpPath) {
    CopyPropertyText(appName, MAX_PATH, lpPath);
  }

  // 実行アプリ名（視聴無し）
  void GetAppNameNP(LPWSTR lpPath, SIZE_T MAX_SIZE) {
    CopyPropertyText(lpPath, MAX_SIZE, appName2);
  }
  void SetAppNameNP(LPCWSTR lpPath) {
    CopyPropertyText(appName2, MAX_PATH, lpPath);
  }

  // 実行アプリ・オプション（視聴有り）
  void GetAppOption(LPWSTR lpPath, SIZE_T MAX_SIZE) {
    CopyPropertyText(lpPath, MAX_SIZE, appOpt);
  }
  void SetAppOption(LPCWSTR lpPath) {
    CopyPropertyText(appOpt, MAX_PATH, lpPath);
  }

  // 実行アプリ・オプション（視聴無し）
  void GetAppOptionNP(LPWSTR lpPath, SIZE_T MAX_SIZE) {
    CopyPropertyText(lpPath, MAX_SIZE, appOpt2);
  }
  void SetAppOptionNP(LPCWSTR lpPath) {
    CopyPropertyText(appOpt2, MAX_PATH, lpPath);
  }

  // 録画ファイル拡張子
  void GetFileExtension(LPWSTR lpExt, SIZE_T sz) {
    CopyPropertyText(lpExt, sz, fileExt);
  }
  void SetFileExtension(LPCWSTR lpExt) {
    CopyPropertyText(fileExt, MAX_PATH, lpExt);
  }

  // 録画先フォルダ
  void GetRecordPath(LPWSTR lpPath, SIZE_T MAX_SIZE) {
    CopyPropertyText(lpPath, MAX_SIZE, filePath);
  }
  void SetRecordPath(LPCWSTR lpPath) {
    CopyPropertyText(filePath, MAX_PATH, lpPath);
  }

  // 局情報
  StationInfo *CreateNewStationInfo() { return (new myStationInfo); }
  bool GetStationInfo(const int no, StationInfo *si) {
    if (!si || no < 0 || no >= STATION_MAX || !stations[no].name[0])
      return false;
    si->SetName(stations[no].name);
    si->SetChannel(stations[no].channel);
    si->SetServiceID(stations[no].service);
    return true;
  }
  bool SetStationInfo(StationInfo *si) {
    if (!si)
      return false;
    WCHAR name[STATION_NAME_MAX] = {};
    si->GetName(name, _countof(name));
    name[_countof(name) - 1] = 0;
    if (!name[0])
      return false;
    for (int no = 0; no < STATION_MAX; ++no) {
      if (!stations[no].name[0] || wcscmp(stations[no].name, name) == 0) {
        stations[no].SetName(name);
        stations[no].channel = si->GetChannel();
        stations[no].service = si->GetServiceID();
        return true;
      }
    }
    return false;
  }
};

class myRockStatus : public RockStatus {
  std::atomic<DWORD> staSmartCard;
  std::atomic<float> statSignal;
  std::atomic<DWORD> staError;
  std::atomic<DWORD> staScrambling;
  std::atomic<DWORD> staBitrate;

public:
  myRockStatus()
      : staSmartCard(0), statSignal(0.0f), staError(0), staScrambling(0),
        staBitrate(0) {}

  // ステータス・信号レベル
  void SetStatusSmartCard(const DWORD val) { staSmartCard = val; }
  DWORD GetStatusSmartCard() const { return staSmartCard; }

  // ステータス・信号レベル
  void SetStatusSignalLevel(const float val) { statSignal = val; }
  float GetStatusSignalLevel() const { return statSignal; }

  // ステータス・ビットレート
  void SetStatusBitrate(const DWORD val) { staBitrate = val; }
  DWORD GetStatusBitrate() const { return staBitrate; }

  // ステータス・エラーパケット
  void SetStatusErrorPacket(const DWORD val) { staError = val; }
  DWORD GetStatusErrorPacket() const { return staError; }

  // ステータス・複合漏れ
  void SetStatusScramblingPacket(const DWORD val) { staScrambling = val; }
  DWORD GetStatusScramblingPacket() const { return staScrambling; }
};

static TvRockDTV *activeReceiver = nullptr;

// このアダプターはレガシーなC++ ABIの変換のみを行います。
// フィルタリング、バッファリング、ソケットの所有権管理、および送信／ステータス関連の操作は
// すべてRustで実装されています。
class RustStreamRequest : public RockRequest_V103 {
  std::shared_ptr<void> sender;
  static bool Narrow(LPCWSTR text,std::string &out,size_t maximum) {
    if(!text)return false;out.clear();
    for(size_t i=0;text[i];++i){if(i>=maximum || text[i]>127)return false;out.push_back(static_cast<char>(text[i]));}
    return true;
  }
public:
  RustStreamRequest(DWORD options=0):sender(udp_new(options),udp_free){}
  bool Valid()const{return sender!=nullptr;}
  void Reset(WORD service){udp_reset(sender.get(),service);}
  void Service(WORD service,bool changed){if(changed)Reset(service);else udp_service(sender.get(),service);}
  bool Feed(const BYTE *p,size_t n){return udp_feed(sender.get(),p,n);}
  void PowerOff()override{} bool ChannelChange(DWORD,DWORD)override{return false;}
  bool RecordStart(LPWSTR)override{return false;}bool QuickRecord(LPWSTR)override{return false;}
  bool RecordStop()override{return false;}void ScreenCapture()override{}void OnOption()override{}void CollectSetting()override{}
  bool StreamStart(LPCWSTR address,WORD port)override{return StreamStartEx(address,port,7);}
  bool StreamStartEx(LPCWSTR address,WORD port,DWORD flags)override{return StreamStartSession(L"",address,port,flags);}
  bool StreamStop()override{return StreamStopSession(L"");}
  bool GetStreamStatus(StreamStatus *status)override{return GetStreamSessionStatus(L"",status);}
  bool StreamStartSession(LPCWSTR session,LPCWSTR address,WORD port,DWORD flags)override {
    std::string token,ip;return Narrow(session,token,64)&&Narrow(address,ip,15)&&udp_start(sender.get(),token.c_str(),ip.c_str(),port,flags);
  }
  bool Srt(LPCWSTR session) {std::string token;return Narrow(session,token,64)&&stream_is_srt(sender.get(),token.c_str());}
  bool StartSrt(LPCWSTR session,LPCWSTR address,WORD port,DWORD flags,DWORD latency,LPCWSTR secret) {
    std::string token,ip,key;
    return Narrow(session,token,64)&&Narrow(address,ip,15)&&Narrow(secret,key,79)&&stream_start_srt(sender.get(),token.c_str(),ip.c_str(),port,flags,latency,key.c_str());
  }
  bool Pace(LPCWSTR session) {std::string token;return Narrow(session,token,64)&&udp_pace(sender.get(),token.c_str());}
  bool Paced(LPCWSTR session) {std::string token;return Narrow(session,token,64)&&udp_is_paced(sender.get(),token.c_str());}
  bool StreamStopSession(LPCWSTR session)override {std::string token;return Narrow(session,token,64)&&udp_stop(sender.get(),token.c_str());}
  bool GetStreamSessionStatus(LPCWSTR session,StreamStatus *status)override {
    if(!status||status->Size!=sizeof(*status))return false;
    std::string token;RustStreamStatus raw={};
    if(!Narrow(session,token,64)||!udp_status(sender.get(),token.c_str(),&raw))return false;
    *status={};status->Size=sizeof(*status);status->Enabled=raw.enabled;
    for(size_t i=0;i<sizeof(raw.address);++i)status->Address[i]=static_cast<unsigned char>(raw.address[i]);
    status->Port=raw.port;status->Service=raw.service;status->Flags=raw.flags;
    status->SentBytes=raw.bytes;status->LastSendTick=raw.lastTick;status->LastError=raw.error;return true;
  }
};
static std::mutex sourceMutex;
static TvRockDTV *sourceOwner=nullptr;
static DWORD sourceMode=STREAM_MODE_OFF;
static std::shared_ptr<RustStreamRequest> activeSource;
static std::shared_ptr<RustStreamRequest> Source(TvRockDTV *owner,DWORD mode=0) {
  std::lock_guard<std::mutex> lock(sourceMutex);
  return sourceOwner==owner && (!mode||sourceMode==mode)?activeSource:nullptr;
}
static std::shared_ptr<RustStreamRequest> ClearSource(TvRockDTV *owner) {
  std::lock_guard<std::mutex> lock(sourceMutex);
  if(sourceOwner!=owner)return {};
  sourceOwner=nullptr;sourceMode=STREAM_MODE_OFF;return std::move(activeSource);
}
static_assert(sizeof(RustStreamStatus)==56,"Rust stream status ABI");
struct StreamLease { ULONGLONG touched=0; DWORD grace=15000; };
struct RemoteState {
  std::shared_ptr<RustStreamRequest> librarySource;
  std::map<std::wstring,StreamLease> streamLeases;
  std::recursive_mutex callbackMutex;
  std::atomic<bool> stopping{false};
  RockRequest *request;
  std::wstring streamSession;
  DWORD streamError=0;
  ULONGLONG lastStreamControl=0;
  DWORD streamGraceMs=15000;
  DWORD streamFlags=7;
  bool sibling = false;
  std::atomic<bool> playback{false};
  std::atomic<bool> graphSupported{false}, graphReady{false}, idleSupported{false};
  std::atomic<DWORD> receiverService{0};
  std::atomic<bool> receiverReady{false};
  std::atomic<bool> qualitySupported{false};
  std::atomic<DWORD> receiverDrops{0};
  std::atomic<ULONGLONG> qualityTick{0}, receiverEpoch{0};
  std::atomic<ULONGLONG> receivedBytes{0}, lastReceiveTick{0};
  std::shared_ptr<void> client;
  HMODULE module = nullptr;
  HANDLE thread = nullptr;
  unsigned threadId = 0;
  HANDLE ready = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  RemoteState(RockRequest *p, std::shared_ptr<void> c)
      : request(p), client(std::move(c)) {}
  ~RemoteState() {
    if (thread)
      CloseHandle(thread);
    if (ready)
      CloseHandle(ready);
  }
};

class myTvRockDTV : public TvRockDTV {

public:
  enum {
    MODE_NONE = 0,
    MODE_CURRENT_INFO,
  };

protected:
public:
  static RockProperty *pProp;
  static RockStatus *pStat;
  static int epgMode;
  static DWORD dwDtvCh;
  static DWORD dwDtvServiceID;
  static char dtvServerUrl[1024];
  static char dtvPrivateID[1024];
  static WCHAR dtDriverName[1024];
  RockRequest *pRockReq;

public:
  myTvRockDTV()
      : pRockReq(NULL), __hInst(NULL), deviceID(0), isTrue(false), oldCh(0),
        oldServiceID(0), eventsReady(false), core(nullptr) {}

  ~myTvRockDTV() {
    Dispose();
    if (myPtr == this)
      myPtr = NULL;
  }

  bool Initialize(const DWORD dwDevID, RockRequest *pReq) try {
    std::lock_guard<std::recursive_mutex> guard(streamMutex);
    if (isTrue || activeReceiver || !pReq)
      return false;
    isTrue = false;

    deviceID = dwDevID;

    pRockReq = pReq;
    activeReceiver = this;

#ifdef _DEBUG
//			trace(L"TvRockDTV::initailize. (%s)\n", GetClassName());
#endif

    deviceID = dwDevID;
    dwDtvCh = 0;
    dwDtvServiceID = 0;
    if(auto source=Source(this))source->Reset(0);
    oldCh = 0;
    oldServiceID = 0;

    pProp = new (std::nothrow) myRockProperty(deviceID);
    pStat = new (std::nothrow) myRockStatus();
    if (!pProp || !pStat) {
      Dispose();
      return false;
    }
    epgMode = MODE_NONE;

    core = tr_new();
    if (!core) {
      Dispose();
      return false;
    }
    eventsReady = true;

    dtvServerUrl[0] = dtvPrivateID[0] = 0;
    dtDriverName[0] = 0;
    char parentCode[1024] = {};
    bool hasParent = false;
    bool invalidParent = false;
    {
      int argc = 0;
      LPTSTR *argv = CommandLineToArgvW(GetCommandLine(), &argc);
      for (int i = 1; argv && i < argc; i++) {
        if (wcscmp(L"/parent", argv[i]) == 0) {
          if (i + 1 >= argc || !argv[i + 1][0] || argv[i + 1][0] == L'/' ||
              !WideCharToMultiByte(CP_UTF8, 0, argv[i + 1], -1, parentCode,
                                   sizeof(parentCode), NULL, NULL)) {
            invalidParent = true;
            break;
          }
          hasParent = true;
          i++;
          continue;
        }
        if (i + 1 >= argc)
          continue;
        if (wcscmp(L"/d", argv[i]) == 0) {
          memset(dtDriverName, 0, sizeof(dtDriverName));
          wcsncpy_s(dtDriverName, 1024, argv[i + 1], _TRUNCATE);
          i++;
        }

        else if (wcscmp(L"/host", argv[i]) == 0) {
          memset(dtvServerUrl, 0, sizeof(dtvServerUrl));
          if (!WideCharToMultiByte(CP_UTF8, 0, argv[i + 1], -1, dtvServerUrl,
                                   sizeof(dtvServerUrl), NULL, NULL))
            dtvServerUrl[0] = 0;
          dtvServerUrl[sizeof(dtvServerUrl) - 1] = 0;
          i++;
        }

        else if (wcscmp(L"/pid", argv[i]) == 0) {
          memset(dtvPrivateID, 0, sizeof(dtvPrivateID));
          if (!WideCharToMultiByte(CP_UTF8, 0, argv[i + 1], -1, dtvPrivateID,
                                   sizeof(dtvPrivateID), NULL, NULL))
            dtvPrivateID[0] = 0;
          dtvPrivateID[sizeof(dtvPrivateID) - 1] = 0;
          i++;
        }
      }
      if (argv)
        LocalFree(argv);
    }
    if (invalidParent) {
      Dispose();
      return false;
    }

    activeClient =
        std::shared_ptr<void>(dtv_new(dtvPrivateID, dtvServerUrl,
                                     hasParent ? parentCode : nullptr), dtv_free);
    if (activeClient && !hasParent)
      LoadDriverCh(dtDriverName);
    if (!activeClient || !dtv_wait_ready(activeClient.get(), 20000)) {
      Dispose();
      return false;
    }
    isTrue = true;
    generation = 0;

    remote = std::make_shared<RemoteState>(pReq, activeClient);
    remote->sibling = hasParent;
    if(hasParent){ClearSource(this);} else {remote->librarySource=Source(this);}
    if (!remote->ready) {
      Dispose();
      return false;
    }
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                            reinterpret_cast<LPCWSTR>(&RemoteFunc),
                            &remote->module)) {
      Dispose();
      return false;
    }
    auto context = new (std::nothrow) std::shared_ptr<RemoteState>(remote);
    if (context)
      remote->thread = (HANDLE)_beginthreadex(NULL, 0, RemoteFunc, context, 0,
                                              &remote->threadId);
    if (!remote->thread) {
      delete context;
      FreeLibrary(remote->module);
      Dispose();
      return false;
    }

    SetEvent(remote->ready);

    return TRUE;
  }

  catch (...) {
    Dispose();
    return false;
  }

  void Dispose() {
    auto retiringSource=ClearSource(this);
    std::shared_ptr<RemoteState> state;
    {
      std::lock_guard<std::recursive_mutex> guard(streamMutex);
      state = remote;
    }
    if (state) {
      state->stopping = true;
      dtv_stop(state->client.get());
      SetEvent(state->ready);
      std::lock_guard<std::recursive_mutex> callbackGuard(state->callbackMutex);
      state->librarySource.reset();
      state->request = nullptr;
    }
    {
      std::lock_guard<std::recursive_mutex> guard(streamMutex);
      isTrue = false;
      if (activeReceiver != this)
        return;
      if (activeClient)
        dtv_stop(activeClient.get());
      if (eventsReady) {
        tr_free(core);
        core = nullptr;
        eventsReady = false;
      }
      delete pProp;
      pProp = nullptr;
      delete pStat;
      pStat = nullptr;
      pRockReq = nullptr;
      activeReceiver = nullptr;
      activeClient.reset();
      remote.reset();
    }
    // コールバックを待機している間、あるいはそのコールバック自身のスレッドをjoinする際には
    // 決してstreamMutexを保持してはならない。
    if (state && state->thread && state->threadId != GetCurrentThreadId())
      WaitForSingleObject(state->thread, INFINITE);
  }

  void ResetPlayback() {
    std::lock_guard<std::recursive_mutex> guard(streamMutex);
    if (isTrue) {
      tr_reset(core,dwDtvCh,static_cast<unsigned short>(dwDtvServiceID),deviceID);
      epgMode=MODE_CURRENT_INFO;
    }
  }

  bool ProcessTS(BYTE *Buffer, const DWORD Length) try {
    std::lock_guard<std::recursive_mutex> guard(streamMutex);
    if (!isTrue)
      return true;
    if (!Buffer && Length)
      return false;
    if (!Length)
      return true;
    if (remote && remote->sibling) {
      remote->receivedBytes += Length;
      remote->lastReceiveTick = GetTickCount64();
    }
    if(auto source=Source(this,STREAM_MODE_PROCESS_TS))source->Feed(Buffer,Length);
    if (epgMode == MODE_NONE) {
      tr_reset(core, dwDtvCh, static_cast<unsigned short>(dwDtvServiceID),
               deviceID);
      epgMode = MODE_CURRENT_INFO;
    }
    if (!tr_process(core, Buffer, Length, RustSend))
      return false;
    tr_status(core, pStat->GetStatusSmartCard(), pStat->GetStatusSignalLevel(),
              pStat->GetStatusBitrate(), pStat->GetStatusErrorPacket(),
              pStat->GetStatusScramblingPacket(), RustSend);
    const uint64_t connectedGeneration = dtv_generation(activeClient.get());
    if (connectedGeneration && connectedGeneration != generation) {
      generation = connectedGeneration;
      tr_resend(core);
    }
    tr_flush(core, static_cast<unsigned short>(dwDtvServiceID), RustSend,
             RustValid);

    return true;
  }

  catch (...) {
    return false;
  }

  bool OnStreamChange(const DWORD Channel, const DWORD ServiceID) {
    std::lock_guard<std::recursive_mutex> guard(streamMutex);
    if (!isTrue)
      return FALSE;

    if(auto source=Source(this))source->Service(static_cast<WORD>(ServiceID),dwDtvCh!=Channel || dwDtvServiceID!=ServiceID);
    if(remote && remote->sibling && (dwDtvCh!=Channel || dwDtvServiceID!=ServiceID))++remote->receiverEpoch;
    dwDtvCh = Channel;
    dwDtvServiceID = ServiceID;

    if (dwDtvCh != oldCh || dwDtvServiceID != oldServiceID) {
      epgMode = MODE_NONE;
      dtv_reset_service(activeClient.get());
      const std::string change =
          "STC:" + std::to_string(Channel) + "," + std::to_string(ServiceID);
      dtv_send(activeClient.get(), change.c_str());
      oldCh = dwDtvCh;
      oldServiceID = dwDtvServiceID;
    }

    return TRUE;
  }

  // 放送波の時間を返す
  void GetBroadCastTime(SYSTEMTIME *m_Time) {
    if (!m_Time)
      return;
    std::lock_guard<std::recursive_mutex> guard(streamMutex);
    GetLocalTime(m_Time);
    if (!isTrue)
      return;
    const int64_t broadcast = tr_time(core);
    if (broadcast) {
      // SYSTEMTIME represents the Japanese broadcast wall clock.
      ULARGE_INTEGER value;
      value.QuadPart =
          static_cast<ULONGLONG>(broadcast + 9 * 3600 + 11644473600LL) *
          10000000ULL;
      FILETIME ft = {value.LowPart, value.HighPart};
      FileTimeToSystemTime(&ft, m_Time);
    }
  }

public:
  RockProperty *GetProperty() { return pProp; }
  void ReceiverReady(BOOL ready,DWORD service) {
    if (remote && remote->sibling) {
      remote->receiverReady=ready!=FALSE;
      remote->receiverService=service;
    }
  }

  void PlaybackCapabilities(BOOL supported,BOOL ready) {
    if(remote && remote->sibling) {
      remote->graphReady=ready!=FALSE;
      remote->graphSupported=supported!=FALSE;
    }
  }

  void ReceiverQuality(DWORD drops) {
    if(remote && remote->sibling) {
      remote->receiverDrops=drops;
      remote->qualityTick=GetTickCount64();
      remote->qualitySupported=true;
    }
  }

  RockStatus *GetStatus() { return pStat; }

  void Verbose(const BOOL on) { flagVerbose = on; }

public:
  void Trace(LPCWSTR lpszFormat, ...) {
    if (!lpszFormat)
      return;
    WCHAR szTextBuff[1024];
    va_list args;
    va_start(args, lpszFormat);
    _vsnwprintf_s(szTextBuff, _countof(szTextBuff), _TRUNCATE, lpszFormat,
                  args);
    va_end(args);
    OutputDebugString(szTextBuff);
  }

  void Trace(LPCSTR lpszFormat, ...) {
    if (!lpszFormat)
      return;
    char szTextBuff[1024];
    va_list args;
    va_start(args, lpszFormat);
    _vsnprintf_s(szTextBuff, _countof(szTextBuff), _TRUNCATE, lpszFormat, args);
    va_end(args);
    OutputDebugStringA(szTextBuff);
  }

public:
  std::shared_ptr<RemoteState> StreamContext()const{return remote;}
  HINSTANCE __hInst;

private:
  DWORD deviceID;

  bool isTrue;

  DWORD oldCh;
  DWORD oldServiceID;
  std::shared_ptr<RemoteState> remote;
  bool eventsReady;
  void *core;
  uint64_t generation = 0;
};

RockProperty *myTvRockDTV::pProp = NULL;
RockStatus *myTvRockDTV::pStat = NULL;
int myTvRockDTV::epgMode;
DWORD myTvRockDTV::dwDtvCh;
DWORD myTvRockDTV::dwDtvServiceID;
char myTvRockDTV::dtvServerUrl[1024];
char myTvRockDTV::dtvPrivateID[1024];
WCHAR myTvRockDTV::dtDriverName[1024];

static bool RecordingPath(std::wstring &path, bool &changed) {
  changed = false;
  if (path.empty())
    return false;
  const std::wstring original = path;
  const size_t slash = path.find_last_of(L"\\/");
  const size_t dot = path.find_last_of(L'.');
  const bool hasExtension =
      dot != std::wstring::npos && (slash == std::wstring::npos || dot > slash);
  const std::wstring base = hasExtension ? path.substr(0, dot) : path;
  const std::wstring ext = hasExtension ? path.substr(dot) : L"";
  for (int index = 0; index <= 999; ++index) {
    if (index)
      path = base + L"(" + std::to_wstring(index) + L")" + ext;
    if (GetFileAttributesW(path.c_str()) == INVALID_FILE_ATTRIBUTES) {
      DWORD error = GetLastError();
      if (error != ERROR_FILE_NOT_FOUND && error != ERROR_PATH_NOT_FOUND)
        return false;
      changed = path != original;
      return true;
    }
  }
  return false;
}

static RockRequest_V101 *StreamRequest(RemoteState &state) {
  if(state.librarySource)return state.librarySource.get();
  return state.request && (strcmp(state.request->GetClassName(),"RockRequest_V101")==0 || strcmp(state.request->GetClassName(),"RockRequest_V102")==0 || strcmp(state.request->GetClassName(),"RockRequest_V103")==0)
      ? static_cast<RockRequest_V101 *>(state.request) : nullptr;
}

static RockRequest_V102 *StreamOptionsRequest(RemoteState &state) {
  if(state.librarySource)return state.librarySource.get();
  return state.request && (strcmp(state.request->GetClassName(),"RockRequest_V102")==0 || strcmp(state.request->GetClassName(),"RockRequest_V103")==0)
      ? static_cast<RockRequest_V102 *>(state.request) : nullptr;
}

static RockRequest_V103 *MultiStreamRequest(RemoteState &state) {
  if(state.librarySource)return state.librarySource.get();
  return state.request && strcmp(state.request->GetClassName(),"RockRequest_V103")==0
      ? static_cast<RockRequest_V103 *>(state.request) : nullptr;
}
static void SendSessionStatus(RemoteState &state,bool success,const std::wstring &token,DWORD error=0) {
  auto host=MultiStreamRequest(state);
  StreamStatus status={};status.Size=sizeof(status);
  const bool available=host && host->GetStreamSessionStatus(token.c_str(),&status);
  wchar_t buffer[1024];
  swprintf_s(buffer,L"STR:{\"session\":\"%ls\",\"request\":\"%ls\",\"supported\":true,\"optionsSupported\":true,\"multiSupported\":true,\"srtSupported\":%ls,\"transport\":\"%ls\",\"pacedSupported\":%ls,\"paced\":%ls,\"success\":%ls,\"enabled\":%ls,\"address\":\"%ls\",\"port\":%u,\"service\":%u,\"flags\":%lu,\"bytes\":%llu,\"lastSendTick\":%llu,\"error\":%lu}",
      token.c_str(),token.c_str(),state.librarySource?L"true":L"false",state.librarySource&&state.librarySource->Srt(token.c_str())?L"srt":L"udp",state.librarySource?L"true":L"false",state.librarySource&&state.librarySource->Paced(token.c_str())?L"true":L"false",success&&available?L"true":L"false",available&&status.Enabled?L"true":L"false",
      available?status.Address:L"",available?status.Port:0,available?status.Service:0,available?status.Flags:0,
      available?status.SentBytes:0,available?status.LastSendTick:0,available&&status.LastError?status.LastError:error);
  dtv_send_wide(state.client.get(),buffer,wcslen(buffer));
}
static void ReportSessionLeases(RemoteState &state,ULONGLONG now) {
  auto host=MultiStreamRequest(state);if(!host)return;
  for(auto it=state.streamLeases.begin();it!=state.streamLeases.end();) {
    if(now-it->second.touched>it->second.grace && host->StreamStopSession(it->first.c_str())) {
      SendSessionStatus(state,true,it->first,ERROR_TIMEOUT);
      it=state.streamLeases.erase(it);
    } else {SendSessionStatus(state,true,it->first);++it;}
  }
}

static void SendStreamStatus(RemoteState &state, bool success, const std::wstring &requestId) {
  StreamStatus status = {}; status.Size=sizeof(status);
  auto extension=StreamRequest(state);
  const bool supported=extension!=nullptr;
  const bool available=supported && extension->GetStreamStatus(&status);
  // 補間される文字列はすべて16進トークンまたは数値形式のIPアドレスに限定される。
  wchar_t buffer[1024];
  swprintf_s(buffer,L"STR:{\"session\":\"%ls\",\"request\":\"%ls\",\"supported\":%ls,\"optionsSupported\":%ls,\"success\":%ls,\"enabled\":%ls,\"address\":\"%ls\",\"port\":%u,\"service\":%u,\"flags\":%lu,\"bytes\":%llu,\"lastSendTick\":%llu,\"error\":%lu}",
      state.streamSession.c_str(),requestId.c_str(),supported?L"true":L"false",StreamOptionsRequest(state)?L"true":L"false",
      success&&available?L"true":L"false",available&&status.Enabled?L"true":L"false",
      available?status.Address:L"",available?status.Port:0,available?status.Service:0,available?status.Flags:0,
      available?status.SentBytes:0,available?status.LastSendTick:0,available&&status.LastError?status.LastError:state.streamError);
  dtv_send_wide(state.client.get(),buffer,wcslen(buffer));
}

static bool DispatchRemoteEvent(RemoteState &state,
                                const RustRemoteEvent &event,
                                const wchar_t *filename) {
  std::lock_guard<std::recursive_mutex> guard(state.callbackMutex);
  RockRequest *request = state.request;
  if (state.stopping || !request)
    return false;
  const auto ev = event.kind;
  switch (ev) {
  case 0: // Ready、LoopBack、あるいは切断されたクライアント：　これらはチャネルの変更には当たりません
    break;
  case 0x26: {
    // 予約済みのチャネル値はABIを変更することなく既存のコールバックを拡張する
    if (!state.sibling) break;
    const bool success=request->ChannelChange(0xfffffffeUL,event.service);
    if(event.service==0 && success)
      state.idleSupported=request->ChannelChange(0xfffffffdUL,2);
    // 従来のコントロールは従来の確認応答形式および失敗時の挙動を維持します
    if(event.service<2) {
      if(!success)break;
      state.playback=true;
      char ack[96];sprintf_s(ack,"PBA:{\"generation\":%lu}",event.channel);
      dtv_send(state.client.get(),ack);
    } else {
      char ack[192];sprintf_s(ack,"PBA:{\"generation\":%lu,\"action\":\"%s\",\"success\":%s}",
          event.channel,event.service==2?"pause-direct":event.service==3?"resume-direct":"idle",success?"true":"false");
      dtv_send(state.client.get(),ack);
    }
    break;
  }
  case 0x23:
    request->ChannelChange(0,event.service);
    break;
  case 0x25: {
    std::wstring payload(filename);
    std::vector<std::wstring> fields;
    size_t begin=0,pos;
    while((pos=payload.find(L',',begin))!=std::wstring::npos) {fields.push_back(payload.substr(begin,pos-begin));begin=pos+1;}
    fields.push_back(payload.substr(begin));
    if(fields.size()!=5)break;
    bool success=false;
    if(!state.sibling && state.librarySource && (state.streamLeases.count(fields[0])||state.streamLeases.size()<32)) {
      std::wstring secret;
      for(size_t i=0;i+1<fields[4].size();i+=2)secret.push_back(static_cast<wchar_t>(wcstoul(fields[4].substr(i,2).c_str(),nullptr,16)));
      success=state.librarySource->StartSrt(fields[0].c_str(),fields[1].c_str(),static_cast<WORD>(event.service),wcstoul(fields[2].c_str(),nullptr,10),wcstoul(fields[3].c_str(),nullptr,10),secret.c_str());
      SecureZeroMemory(secret.data(),secret.size()*sizeof(wchar_t));
      if(success)state.streamLeases[fields[0]]={GetTickCount64(),(event.channel?event.channel:15)*1000};
    }
    SendSessionStatus(state,success,fields[0],success?ERROR_SUCCESS:ERROR_NOT_SUPPORTED);
    break;
  }
  case 0x24: {
    // 既存のリース済みDLL送信側のみが自身のトランスポートを変更できる。
    // 古いセッションからの遅延したフィードバックに応じてストリームを新規作成したり更新したりしてはならない。
    const std::wstring token(filename);
    const auto lease=state.streamLeases.find(token);
    if(!state.sibling && state.librarySource && lease!=state.streamLeases.end()
        && GetTickCount64()-lease->second.touched<=lease->second.grace) {
      const bool ok=state.librarySource->Pace(token.c_str());
      SendSessionStatus(state,ok,token);
    }
    break;
  }
  case 0x20:
  case 0x21:
  case 0x22: {
    std::wstring payload(filename);
    const auto comma=payload.find(L',');
    const std::wstring session=payload.substr(0,comma);
    auto extension=StreamRequest(state);
    bool success=false;
    if(auto host=MultiStreamRequest(state)) {
      DWORD error=ERROR_SUCCESS;
      if(ev==0x20 && comma!=std::wstring::npos) {
        std::wstring address=payload.substr(comma+1);DWORD flags=7;
        const auto pos=address.find(L',');
        if(pos!=std::wstring::npos) {
          wchar_t *end=nullptr;auto value=wcstoul(address.c_str()+pos+1,&end,10);
          if(!end||*end||value>15)break;
          flags=static_cast<DWORD>(value);if(flags&8)flags|=1;address.resize(pos);
        }
        StreamStatus existing={};existing.Size=sizeof(existing);
        success=host->GetStreamSessionStatus(session.c_str(),&existing) && existing.Enabled && existing.Port==event.service
            && existing.Flags==flags && wcscmp(existing.Address,address.c_str())==0;
        if(!success && (state.streamLeases.count(session)||state.streamLeases.size()<32))
          success=host->StreamStartSession(session.c_str(),address.c_str(),static_cast<WORD>(event.service),flags);
        if(success)state.streamLeases[session]={GetTickCount64(),(event.channel?event.channel:15)*1000};
        else error=ERROR_GEN_FAILURE;
      } else if(ev==0x21) {
        success=host->StreamStopSession(session.c_str());
        if(success)state.streamLeases.erase(session);
      } else if(ev==0x22) {
        success=true;auto it=state.streamLeases.find(session);
        if(it!=state.streamLeases.end())it->second.touched=GetTickCount64();
      }
      SendSessionStatus(state,success,session,error);
      return true;
    }
    if (extension) {
      if (ev==0x20 && comma!=std::wstring::npos) {
        std::wstring address=payload.substr(comma+1);
        DWORD flags=7;
        const auto optionsComma=address.find(L',');
        if (optionsComma!=std::wstring::npos) {
          wchar_t *end=nullptr;
          const auto value=wcstoul(address.c_str()+optionsComma+1,&end,10);
          if (!end || *end || value>15) break;
          flags=static_cast<DWORD>(value); if(flags&8)flags|=1;
          address.resize(optionsComma);
        }
        StreamStatus existing={}; existing.Size=sizeof(existing);
        success=state.streamSession==session && state.streamFlags==flags && extension->GetStreamStatus(&existing)
            && existing.Enabled && existing.Port==event.service && wcscmp(existing.Address,address.c_str())==0;
        auto options=StreamOptionsRequest(state);
        if (!success) success=options ? options->StreamStartEx(address.c_str(),static_cast<WORD>(event.service),flags)
            : flags==7 && extension->StreamStart(address.c_str(),static_cast<WORD>(event.service));
        state.streamFlags=flags;
        if (success) {
          state.streamSession=session;
          state.lastStreamControl=GetTickCount64();
          state.streamGraceMs=(event.channel?event.channel:15)*1000;
        }
        state.streamError=success?ERROR_SUCCESS:(!StreamOptionsRequest(state) && state.streamFlags!=7 ? ERROR_NOT_SUPPORTED : ERROR_GEN_FAILURE);
      } else if (ev==0x21 && state.streamSession.empty()) {
        StreamStatus existing={}; existing.Size=sizeof(existing);
        success=extension->GetStreamStatus(&existing) && !existing.Enabled;
      } else if (ev==0x21 && session==state.streamSession) {
        success=extension->StreamStop();
        if (success) { state.streamSession.clear(); state.streamError=0; }
      } else if (ev==0x22) {
        success=true;
        if (session==state.streamSession) state.lastStreamControl=GetTickCount64();
      }
    }
    SendStreamStatus(state,success,session);
    break;
  }
  case TVROCK_DTV_RECORD_START:
  case TVROCK_DTV_RECORD_START_QUICK: {
    std::wstring path(filename);
    bool changed;
    if (!RecordingPath(path, changed))
      break;
    bool started = ev == TVROCK_DTV_RECORD_START
                       ? request->RecordStart(&path[0])
                       : request->QuickRecord(&path[0]);
    if (started && !state.stopping) {
      dtv_send(state.client.get(), "STA:3");
      if (changed) {
        const std::wstring rename = L"RNF:" + path;
        dtv_send_wide(state.client.get(), rename.data(), rename.size());
      }
    }
    break;
  }
  case TVROCK_DTV_RECORD_STOP:
    if (request->RecordStop() && !state.stopping) {
      dtv_send(state.client.get(), "STA:2");
    }
    break;
  case TVROCK_DTV_POWER_OFF:
    state.stopping = true;
    request->PowerOff();
    return false;
  case TVROCK_DTV_SCREEN_CAPTURE:
    request->ScreenCapture();
    break;
  case TVROCK_DTV_ON_OPTION:
    request->OnOption();
    break;
  case TVROCK_DTV_CHANNEL_CHANGE: {
    const ULONGLONG started = GetTickCount64();
    const DWORD threadId = GetCurrentThreadId();
    wchar_t diagnostic[256];
    swprintf_s(diagnostic, _countof(diagnostic),
               L"[TvRockDTV] ChannelChange begin: channel=%lu service=%lu thread=%lu\n",
               event.channel, event.service, threadId);
    OutputDebugStringW(diagnostic);
    const bool changed = request->ChannelChange(event.channel, event.service);
    const ULONGLONG elapsed = GetTickCount64() - started;
    swprintf_s(diagnostic, _countof(diagnostic),
               L"[TvRockDTV] ChannelChange end: channel=%lu service=%lu thread=%lu result=%ls elapsed_ms=%llu\n",
               event.channel, event.service, threadId,
               changed ? L"success" : L"failure", elapsed);
    OutputDebugStringW(diagnostic);
    break;
  }
  default:
    break;
  }
  return !state.stopping;
}

static unsigned __stdcall RemoteFunc(LPVOID vdParam) {
  HMODULE module;
  {
    std::unique_ptr<std::shared_ptr<RemoteState>> context(
        static_cast<std::shared_ptr<RemoteState> *>(vdParam));
    std::shared_ptr<RemoteState> state = *context;
    context.reset();
    module = state->module;
    WaitForSingleObject(state->ready, INFINITE);
    try {
      std::vector<wchar_t> path(32768);
      ULONGLONG lastReport=0;
      while (!state->stopping) {
        RustRemoteEvent event = {};
        const int result = dtv_next(state->client.get(), &event, path.data(),
                                    path.size(), 100);
        if (result < 0)
          break;
        if (result && !DispatchRemoteEvent(*state, event, path.data()))
          break;
        const auto now=GetTickCount64();
        if (now-lastReport>=1000) {
          lastReport=now;
          std::lock_guard<std::recursive_mutex> guard(state->callbackMutex);
          if (!state->stopping && state->request) {
            if (state->sibling) {
              char report[512];
              const bool graphSupported=state->graphSupported.load();
              const bool graphReady=graphSupported && state->graphReady.load();
              const auto last=state->lastReceiveTick.load(), quality=state->qualityTick.load();
              sprintf_s(report,"RCV:{\"controlVersion\":3,\"idleSupported\":%s,\"graphPauseSupported\":%s,\"graphPauseReady\":%s,\"playback\":%s,\"ready\":%s,\"bytes\":%llu,\"ageMs\":%llu,\"service\":%lu,\"qualitySupported\":%s,\"drops\":%lu,\"qualityAgeMs\":%llu,\"epoch\":%llu}",
                  state->idleSupported?"true":"false",graphSupported?"true":"false",graphReady?"true":"false",state->playback?"true":"false",state->receiverReady?"true":"false",state->receivedBytes.load(),last?now-last:ULLONG_MAX,state->receiverService.load(),
                  state->qualitySupported?"true":"false",state->receiverDrops.load(),quality?now-quality:ULLONG_MAX,state->receiverEpoch.load());
              dtv_send(state->client.get(),report);
            } else if(MultiStreamRequest(*state)) {
              ReportSessionLeases(*state,now);
            } else if (auto extension=StreamRequest(*state)) {
              // ストリームはこちらが送信する報告によってではなく、受信するステータス・ポーリングによってリースされる。
              // これによりハーフオープン状態の制御用ソケットも検知できる              
              if (!state->streamSession.empty() && now-state->lastStreamControl>state->streamGraceMs) {
                if (extension->StreamStop()) {
                  state->streamSession.clear(); state->streamError=ERROR_TIMEOUT;
                }
              }
              SendStreamStatus(*state,true,L"");
            }
          }
        }
      }
    } catch (...) {
      state->stopping = true;
    }
  } // Destroy C++ thread context before releasing the code module.

  // UCRTの_beginthreadexは、_endthreadexが呼び出されるまで自身のモジュール参照を保持し続ける。
  // CRTを経由して制御を戻すことでその参照およびスレッド状態が確実に解放されるようにする。
  // ここでFreeLibraryAndExitThreadを使用すると、そうしたクリーンアップ処理がバイパスされDLLのメモリリークを引き起こすことになる。  
  FreeLibrary(module);
  return 0;
}

} // namespace TvRock

_DLL_EXPORT bool TvRockDTVSetStreamMode(TvRockDTV *receiver,DWORD mode) {
  if((mode&~0x103U) || (mode&3)>STREAM_MODE_DIRECT_TS)return false;
  std::shared_ptr<RemoteState> state;
  {
    std::lock_guard<std::recursive_mutex> lock(streamMutex);
    if(!receiver||receiver!=myPtr)return false;
    state=static_cast<myTvRockDTV *>(receiver)->StreamContext();
  }
  std::unique_lock<std::recursive_mutex> callback;
  if(state){callback=std::unique_lock<std::recursive_mutex>(state->callbackMutex);if(state->stopping)return false;if(state->sibling)mode=STREAM_MODE_OFF;}
  if(state && !state->librarySource && (!state->streamSession.empty()||!state->streamLeases.empty()))return false;
  std::lock_guard<std::recursive_mutex> modeGuard(streamMutex);
  if(receiver!=myPtr || (state&&state->stopping))return false;
  auto replacement=(mode&3)?std::make_shared<RustStreamRequest>(mode):nullptr;
  if(replacement && !replacement->Valid())return false;
  auto retired=ClearSource(receiver);
  {
    std::lock_guard<std::mutex> lock(sourceMutex);
    sourceOwner=receiver;sourceMode=mode&3;activeSource=replacement;
  }
  if(state){state->streamLeases.clear();state->librarySource=replacement;}
  if(replacement)replacement->Reset(static_cast<WORD>(myTvRockDTV::dwDtvServiceID));
  return true;
}
_DLL_EXPORT bool TvRockDTVStreamTS(TvRockDTV *receiver,const BYTE *data,DWORD size) {
  auto source=Source(receiver,STREAM_MODE_DIRECT_TS);
  return !source||source->Feed(data,size);
}
_DLL_EXPORT void TvRockDTVPlaybackReset(TvRockDTV *receiver) {
  std::lock_guard<std::recursive_mutex> guard(streamMutex);
  if(receiver && receiver==activeReceiver) static_cast<myTvRockDTV *>(receiver)->ResetPlayback();
}
_DLL_EXPORT void TvRockDTVStreamReset(TvRockDTV *receiver) {if(auto source=Source(receiver))source->Reset(0);}
_DLL_EXPORT bool TvRockDTVStreamStart(TvRockDTV *receiver,LPCWSTR address,WORD port,DWORD flags,LPCWSTR session) {
  auto source=Source(receiver);return source&&source->StreamStartSession(session?session:L"",address,port,flags);
}
_DLL_EXPORT bool TvRockDTVStreamStop(TvRockDTV *receiver,LPCWSTR session) {
  auto source=Source(receiver);return source&&source->StreamStopSession(session?session:L"");
}
_DLL_EXPORT bool TvRockDTVGetStreamStatus(TvRockDTV *receiver,StreamStatus *status,LPCWSTR session) {
  auto source=Source(receiver);return source&&source->GetStreamSessionStatus(session?session:L"",status);
}

// 任意登録：古いプラグインはこの機能を呼び出さないためレガシーな状態のままとなります。
_DLL_EXPORT void TvRockDTVPlaybackCapabilities(TvRockDTV *receiver,BOOL supported,BOOL ready) {
  std::lock_guard<std::recursive_mutex> guard(streamMutex);
  if(receiver && receiver==activeReceiver)
    static_cast<myTvRockDTV *>(receiver)->PlaybackCapabilities(supported,ready);
}

_DLL_EXPORT void TvRockDTVReceiverReady(TvRockDTV *receiver, BOOL ready, DWORD service) {
  std::lock_guard<std::recursive_mutex> guard(streamMutex);
  if (receiver && receiver==activeReceiver) {
    auto concrete=static_cast<myTvRockDTV *>(receiver);
    concrete->ReceiverReady(ready,service);
  }
}

_DLL_EXPORT void TvRockDTVReceiverQuality(TvRockDTV *receiver,DWORD drops) {
  std::lock_guard<std::recursive_mutex> guard(streamMutex);
  if(receiver && receiver==activeReceiver)static_cast<myTvRockDTV *>(receiver)->ReceiverQuality(drops);
}

_DLL_EXPORT TvRockDTV *CreateTvRockDTV(HINSTANCE hInst) {
  std::lock_guard<std::recursive_mutex> guard(streamMutex);
  if (myPtr)
    return nullptr;
  myPtr = new (std::nothrow) myTvRockDTV();
  if (myPtr)
    static_cast<myTvRockDTV *>(myPtr)->__hInst = hInst;
  return myPtr;
}

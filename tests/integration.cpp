#include "TvRockDTV.cpp"
#include <cassert>
#include <sstream>
using namespace TvRock;
static void TestDriverChEncoding() {
  const std::wstring source = L"; チャンネル設定\r\n\r\n吊,日本語,0,101\n最終行";
  const std::wstring expected =
      L"CH2:; チャンネル設定\n|\n|吊,日本語,0,101\n|最終行\n";
  for (const UINT codePage : {932u, static_cast<UINT>(CP_UTF8)}) {
    const int size = WideCharToMultiByte(codePage, 0, source.data(),
                                         static_cast<int>(source.size()),
                                         nullptr, 0, nullptr, nullptr);
    assert(size > 0);
    std::string bytes(size, '\0');
    assert(WideCharToMultiByte(codePage, 0, source.data(),
                               static_cast<int>(source.size()), &bytes[0], size,
                               nullptr, nullptr) == size);
    if (codePage == CP_UTF8)
      bytes.insert(0, "\xef\xbb\xbf");
    std::istringstream input(bytes);
    assert(ReadDriverCh(input) == expected);
  }
  std::string bytes("\xff\xfe", 2);
  for (wchar_t ch : source) {
    bytes += static_cast<char>(ch & 0xff);
    bytes += static_cast<char>(ch >> 8);
  }
  std::istringstream utf16(bytes);
  assert(ReadDriverCh(utf16) == expected);
  for (const std::string data : {std::string(), std::string("\xff\xfe", 2),
                                 std::string("\xef\xbb\xbf", 3)}) {
    std::istringstream input(data);
    assert(ReadDriverCh(input).empty());
  }
  std::istringstream trailingNewline("a\r\n\r\n");
  assert(ReadDriverCh(trailingNewline) == L"CH2:a\n|\n");
  std::istringstream shortInput("a");
  assert(ReadDriverCh(shortInput) == L"CH2:a\n");
  bytes += '\0';
  std::istringstream truncated(bytes);
  assert(ReadDriverCh(truncated).empty());
}
static std::vector<std::string> messages;
static bool send(const char *s) {
  messages.emplace_back(s);
  return true;
}
struct Request : RockRequest {
  DWORD channel = 99, service = 99;
  int records = 0, captures = 0;
  std::wstring filename;
  void PowerOff() override {}
  bool ChannelChange(DWORD c, DWORD s) override {
    channel = c;
    service = s;
    return true;
  }
  bool RecordStart(LPWSTR p) override {
    filename = p;
    ++records;
    return true;
  }
  bool QuickRecord(LPWSTR p) override { return RecordStart(p); }
  bool RecordStop() override { return true; }
  void ScreenCapture() override { ++captures; }
  void OnOption() override {}
  void CollectSetting() override {}
};
template<class Base> struct StreamHost : Base {
    StreamStatus status={};int starts=0;DWORD flags=7;
    void PowerOff() override {} bool ChannelChange(DWORD,DWORD) override{return true;}
    bool RecordStart(LPWSTR) override{return true;}bool QuickRecord(LPWSTR) override{return true;}
    bool RecordStop() override{return true;}void ScreenCapture() override{}void OnOption() override{}void CollectSetting() override{}
    bool StreamStart(LPCWSTR address,WORD port) override{return StreamStartEx(address,port,7);}
    bool StreamStartEx(LPCWSTR address,WORD port,DWORD f){++starts;flags=f;status={};status.Size=sizeof(status);status.Enabled=TRUE;status.Port=port;status.Flags=f;wcscpy_s(status.Address,address);return true;}
    bool StreamStop() override{status.Enabled=FALSE;return true;}
    bool GetStreamStatus(StreamStatus *out) override{*out=status;out->Size=sizeof(*out);return true;}
};
static void TestStreamOptionsABI() {
    const std::wstring token(64,L'a');
    StreamHost<RockRequest_V102> host;RemoteState state(&host,{});
    RustRemoteEvent event{0x20,15,1234};
    assert(DispatchRemoteEvent(state,event,(token+L",127.0.0.1,11").c_str()));
    assert(host.starts==1 && host.flags==11 && state.streamFlags==11);
    assert(DispatchRemoteEvent(state,event,(token+L",127.0.0.1,11").c_str()));assert(host.starts==1);
    assert(DispatchRemoteEvent(state,event,(token+L",127.0.0.1,7").c_str()));assert(host.starts==2 && host.flags==7);
    event={0x21,0,0};assert(DispatchRemoteEvent(state,event,token.c_str()));assert(!host.status.Enabled);
    StreamHost<RockRequest_V101> legacy;RemoteState old(&legacy,{});event={0x20,15,1234};
    assert(DispatchRemoteEvent(old,event,(token+L",127.0.0.1,11").c_str()));assert(legacy.starts==0 && old.streamError==ERROR_NOT_SUPPORTED);
    assert(DispatchRemoteEvent(old,event,(token+L",127.0.0.1").c_str()));assert(legacy.starts==1 && legacy.flags==7);
    puts("PASS: V101 default streaming and V102 option dispatch/idempotence");
}

struct MultiStreamHost : StreamHost<RockRequest_V103> {
    std::map<std::wstring,StreamStatus> streams;
    bool StreamStartSession(LPCWSTR token,LPCWSTR address,WORD port,DWORD flags) override {
        auto &s=streams[token];s={};s.Size=sizeof(s);s.Enabled=TRUE;s.Port=port;s.Flags=flags;wcscpy_s(s.Address,address);++starts;return true;
    }
    bool StreamStopSession(LPCWSTR token) override {streams.erase(token);return true;}
    bool GetStreamSessionStatus(LPCWSTR token,StreamStatus *out) override {
        *out={};out->Size=sizeof(*out);auto it=streams.find(token);if(it!=streams.end())*out=it->second;return true;
    }
};
static void TestMultiStreamABI() {
    MultiStreamHost host;RemoteState state(&host,{});
    const std::wstring a(64,L'a'),b(64,L'b'),old(64,L'c');
    RustRemoteEvent event{0x20,15,1234};
    assert(DispatchRemoteEvent(state,event,(a+L",127.0.0.1,7").c_str()));
    assert(DispatchRemoteEvent(state,event,(b+L",127.0.0.2,1").c_str()));
    assert(host.starts==2 && state.streamLeases.size()==2 && host.streams.size()==2);
    assert(DispatchRemoteEvent(state,event,(a+L",127.0.0.1,7").c_str()));assert(host.starts==2);
    event={0x21,0,0};assert(DispatchRemoteEvent(state,event,old.c_str()));assert(host.streams.size()==2);
    assert(DispatchRemoteEvent(state,event,a.c_str()));assert(host.streams.size()==1 && host.streams.count(b));
    event={0x22,0,0};auto before=state.streamLeases[b].touched;
    assert(DispatchRemoteEvent(state,event,b.c_str()));assert(state.streamLeases[b].touched>=before);
    // Only the expired lease stops; a status poll for another session cannot renew it.
    event={0x20,15,1234};assert(DispatchRemoteEvent(state,event,(a+L",127.0.0.1,7").c_str()));
    auto now=GetTickCount64();state.streamLeases[a]={now-16000,15000};state.streamLeases[b]={now,15000};
    ReportSessionLeases(state,now);
    assert(host.streams.size()==1 && host.streams.count(b) && !state.streamLeases.count(a));
    puts("PASS: V103 independent multi-peer start/stop/status/idempotence/lease expiry");
}

int main() {
  TestStreamOptionsABI();
  TestMultiStreamABI();
  TestDriverChEncoding();
  for (int cycle = 0; cycle < 3; ++cycle) {
    void *core = tr_new();
    assert(core);
    tr_reset(core, 100 + cycle, 101, 7);
    BYTE p[188];
    memset(p, 255, 188);
    BYTE tdt[] = {0x47, 0x40, 0x14, 0x10, 0, 0x70, 0x70,
                  5,    0xeb, 0x96, 0x12, 0, 0};
    memcpy(p, tdt, sizeof(tdt));
    assert(tr_process(core, p, 17, send));
    assert(tr_time(core) == 0);
    assert(tr_process(core, p + 17, 171, send));
    assert(tr_time(core) > 0);
    messages.clear();
    tr_status(core, 0, 12.5f, 0, 0, 0, send);
    assert(messages.size() == 2);
    assert(!tr_process(core, nullptr, 1, send));
    tr_free(core);
  }
  Request request;
  RemoteState state(&request, {});
  RustRemoteEvent event{TVROCK_DTV_CHANNEL_CHANGE, 0, 16};
  assert(DispatchRemoteEvent(state, event, L""));
  assert(request.channel == 0 && request.service == 16 && request.records == 0);
  event = {TVROCK_DTV_RECORD_START, 0, 0};
  assert(DispatchRemoteEvent(state, event, L"__tvrock_test_日本語.ts"));
  assert(request.records == 1);
  assert(request.filename.find(L"日本語") != std::wstring::npos);
  state.stopping = true;
  event = {TVROCK_DTV_SCREEN_CAPTURE, 0, 0};
  assert(!DispatchRemoteEvent(state, event, L""));
  assert(request.captures == 0);
  puts("Rust core and typed C++ callback ABI tests passed");
}

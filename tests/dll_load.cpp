#include <winsock2.h>
#include "TvRockDTV.h"
#include <atomic>
#include <cassert>
#include <cstdio>
#include <string>
struct Request : TvRock::RockRequest {
  TvRock::TvRockDTV *object = nullptr;
  std::atomic<int> records{0}, captures{0}, changes{0}, stops{0};
  std::atomic<int> options{0};
  std::atomic<bool> poweredOff{false};
  std::atomic<bool> allowPowerOff{true};
  void PowerOff() override {
    while (!allowPowerOff)
      Sleep(1);
    auto *p = object;
    object = nullptr;
    delete p;
    poweredOff = true;
  }
  bool ChannelChange(DWORD ch, DWORD sid) override {
    assert(ch == 0 && sid == 16);
    assert(object->OnStreamChange(ch, sid));
    ++changes;
    return true;
  }
  bool RecordStart(LPWSTR path) override {
    assert(std::wstring(path).find(records == 0 ? L"第一" : L"第二") !=
           std::wstring::npos);
    ++records;
    return true;
  }
  bool QuickRecord(LPWSTR path) override { return RecordStart(path); }
  bool RecordStop() override {
    ++stops;
    return true;
  }
  void ScreenCapture() override { ++captures; }
  void OnOption() override { ++options; }
  void CollectSetting() override {}
};
int main(int argc, char **argv) {
  const bool selfDelete = argc > 1 && strcmp(argv[1], "self-delete") == 0;
  bool sibling = false;
  for (int i = 1; i < argc; ++i)
    sibling |= strcmp(argv[i], "/parent") == 0;
  assert(GetFileAttributesW(L"dtvclient.dll") == INVALID_FILE_ATTRIBUTES);
  HMODULE dll = LoadLibraryW(L"TvRockDTV.dll");
  assert(dll);
  assert(!GetModuleHandleW(L"dtvclient.dll"));
  auto create = reinterpret_cast<TvRock::TvRockDTV *(*)(HINSTANCE)>(
      GetProcAddress(dll, "CreateTvRockDTV"));
  assert(create);
  auto *object = create(dll);
  assert(object);
  assert(!create(dll));
  assert(strcmp(object->GetClassName(), "TvRockDTV_V100") == 0);
  auto mode=reinterpret_cast<TvRock::SetStreamModeProc>(GetProcAddress(dll,"TvRockDTVSetStreamMode"));
  auto feed=reinterpret_cast<TvRock::StreamTSProc>(GetProcAddress(dll,"TvRockDTVStreamTS"));
  auto reset=reinterpret_cast<TvRock::StreamResetProc>(GetProcAddress(dll,"TvRockDTVStreamReset"));
  auto start=reinterpret_cast<TvRock::StreamStartProc>(GetProcAddress(dll,"TvRockDTVStreamStart"));
  auto stopStream=reinterpret_cast<TvRock::StreamStopProc>(GetProcAddress(dll,"TvRockDTVStreamStop"));
  auto getStream=reinterpret_cast<TvRock::StreamStatusProc>(GetProcAddress(dll,"TvRockDTVGetStreamStatus"));
  assert(mode&&feed&&reset&&start&&stopStream&&getStream);
  WSADATA wsa={};assert(WSAStartup(MAKEWORD(2,2),&wsa)==0);
  SOCKET receiver=socket(AF_INET,SOCK_DGRAM,0);assert(receiver!=INVALID_SOCKET);
  sockaddr_in address={};address.sin_family=AF_INET;address.sin_addr.s_addr=htonl(INADDR_LOOPBACK);
  assert(bind(receiver,reinterpret_cast<sockaddr *>(&address),sizeof(address))==0);
  int length=sizeof(address);getsockname(receiver,reinterpret_cast<sockaddr *>(&address),&length);
  DWORD timeout=100;setsockopt(receiver,SOL_SOCKET,SO_RCVTIMEO,reinterpret_cast<char *>(&timeout),sizeof(timeout));
  Request request;
  request.allowPowerOff = !(sibling && selfDelete);
  request.object = object;
  for (int cycle = 0; cycle < (selfDelete ? 1 : 3); ++cycle) {
    request.records = 0;
    request.captures = 0;
    request.changes = 0;
    request.stops = 0;
    request.options = 0;
    assert(!mode(object,3));
    const DWORD streamMode=cycle%2?TvRock::STREAM_MODE_DIRECT_TS:TvRock::STREAM_MODE_PROCESS_TS;
    assert(mode(object,streamMode));
    assert(object->Initialize(1, &request));
    assert(!object->Initialize(1, &request));
    auto *property = object->GetProperty();
    assert(property);
    if (sibling)
      assert(!property->UseTS() && !property->UseBS() && !property->UseCS());
    else
      assert(property->UseTS());
    property->SetAppPath(L"日本語のパス");
    wchar_t path[MAX_PATH] = {};
    property->GetAppPath(path, MAX_PATH);
    assert(wcscmp(path, L"日本語のパス") == 0);
    assert(object->OnStreamChange(10, 101));
    if (selfDelete) {
      request.allowPowerOff = true;
      for (int i = 0; i < 500 && !request.poweredOff; ++i)
        Sleep(10);
      assert(request.poweredOff);
      if (sibling)
        assert(request.captures == 1 && request.records == 0 &&
               request.changes == 0 && request.stops == 0 && request.options == 0);
      object = nullptr;
      break;
    }
    BYTE p[188];
    memset(p, 255, 188);
    BYTE tdt[] = {0x47, 0x40, 0x14, 0x10, 0, 0x70, 0x70,
                  5,    0xeb, 0x96, 0x12, 0, 0};
    memcpy(p, tdt, sizeof(tdt));
    assert(object->ProcessTS(p,188));
    SYSTEMTIME t = {};
    object->GetBroadCastTime(&t);
    assert(t.wHour == 12);
    for (int i = 0; i < 500 && (sibling ? request.captures == 0 : request.stops == 0); ++i)
      Sleep(10);
    if (sibling)
      assert(request.records == 0 && request.captures == 1 &&
             request.changes == 0 && request.stops == 0 && request.options == 0);
    else
      assert(request.records == 2 && request.captures == 1 &&
             request.changes == 1 && request.stops == 1);
    if(!sibling)assert(start(object,L"127.0.0.1",ntohs(address.sin_port),6,L""));
    else assert(!start(object,L"127.0.0.1",ntohs(address.sin_port),6,L""));
    assert(feed(object,p,188));
    assert(object->ProcessTS(p, 188));
    char received[65536];
    if(!sibling){assert(recv(receiver,received,sizeof(received),0)==188);assert(memcmp(received,p,188)==0);}
    // PROCESS_TS and DIRECT_TS must never send the same packet twice. Sibling
    // instances may receive TS for display/analysis but cannot retransmit it.
    assert(recv(receiver,received,sizeof(received),0)==SOCKET_ERROR);
    if(!sibling){
      TvRock::StreamStatus status={};status.Size=sizeof(status);
      assert(getStream(object,&status,L"")&&status.Enabled&&status.SentBytes==188&&status.Flags==6);
      reset(object);assert(feed(object,p,188));assert(object->ProcessTS(p,188));
      assert(recv(receiver,received,sizeof(received),0)==SOCKET_ERROR);
      assert(object->OnStreamChange(0,16));assert(feed(object,p,188));assert(object->ProcessTS(p,188));
      assert(recv(receiver,received,sizeof(received),0)==188);
      assert(stopStream(object,L""));assert(getStream(object,&status,L"")&&!status.Enabled);
      assert(mode(object,TvRock::STREAM_MODE_OFF));assert(!start(object,L"127.0.0.1",ntohs(address.sin_port),6,L""));
      // Dispose/unload also owns active Rust workers and their destination lock.
      assert(mode(object,TvRock::STREAM_MODE_DIRECT_TS));assert(start(object,L"127.0.0.1",ntohs(address.sin_port),6,L""));
    }
    const auto started = GetTickCount64();
    object->Dispose();
    object->Dispose();
    assert(GetTickCount64() - started < 3000);
  }
  closesocket(receiver);WSACleanup();
  delete object;
  FreeLibrary(dll);
  for (int i = 0; i < 500 && GetModuleHandleW(L"TvRockDTV.dll"); ++i)
    Sleep(10);
  assert(!GetModuleHandleW(L"TvRockDTV.dll"));
  puts("DLL + WebSocket + callbacks + lifecycle + unload tests passed (no "
       "dtvclient.dll)");
}

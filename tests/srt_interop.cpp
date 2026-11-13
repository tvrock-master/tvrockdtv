// Optional real BonDriver_SRT integration. Uses its public BonDriver ABI only.
#include "../compat/rust_core.h"
#include "IBonDriver2.h"
#include <windows.h>
#include <filesystem>
#include <fstream>
#include <vector>
#include <cstdio>
#include <stdexcept>
#include <memory>
#include <string>
namespace fs=std::filesystem;
static void check(bool value,const char *message) {if(!value)throw std::runtime_error(message);}
struct DriverRelease {void operator()(IBonDriver2 *d)const{d->Release();}};
static const char *token="aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
static std::vector<unsigned char> sample() {
  std::vector<unsigned char> expected(1316*256);
  for(size_t i=0;i<expected.size();++i)expected[i]=static_cast<unsigned char>(i*29);
  for(size_t i=0;i<expected.size();i+=188) {
    expected[i]=0x47;expected[i+1]=0x01;expected[i+2]=0x00;expected[i+3]=0x10|((i/188)&15);
  }
  return expected;
}
static void transfer(IBonDriver2 &driver,void *sender,const char *secret) {
  check(stream_start_srt(sender,token,"127.0.0.1",33230,6,120,secret),"start SRT caller");
  check(stream_is_srt(sender,token),"SRT status");
  check(!udp_pace(sender,token),"UDP pacing must not apply to SRT");
  auto expected=sample();
  // Split a TS packet between callbacks, as a real source is allowed to do.
  check(udp_feed(sender,expected.data(),73),"partial input");
  check(udp_feed(sender,expected.data()+73,expected.size()-73),"TS input");
  std::vector<unsigned char> received;
  const auto deadline=GetTickCount64()+8000;
  while(GetTickCount64()<deadline && received.size()<expected.size()) {
    driver.WaitTsStream(50);BYTE *data=nullptr;DWORD size=0,remaining=0;
    check(driver.GetTsStream(&data,&size,&remaining),"read TS");
    if(size)received.insert(received.end(),data,data+size);
  }
  if(received!=expected){RustStreamStatus status{};udp_status(sender,token,&status);fprintf(stderr,"received=%zu expected=%zu sent=%llu enabled=%u error=%u\n",received.size(),expected.size(),status.bytes,status.enabled,status.error);}
  check(received==expected,"BonDriver byte equality");
  RustStreamStatus status{};check(udp_status(sender,token,&status)&&status.bytes==expected.size()&&!status.error,"sender byte counter");
  auto stopped=GetTickCount64();check(udp_stop(sender,token),"stop");
  check(GetTickCount64()-stopped<1000,"bounded stop");
  Sleep(200);
}
static void receiveExpected(IBonDriver2 &driver,const std::vector<unsigned char> &expected) {
  std::vector<unsigned char> received;
  const auto deadline=GetTickCount64()+8000;
  while(GetTickCount64()<deadline && received.size()<expected.size()) {
    driver.WaitTsStream(50);BYTE *data=nullptr;DWORD size=0,remaining=0;
    check(driver.GetTsStream(&data,&size,&remaining),"read surviving receiver");
    if(size)received.insert(received.end(),data,data+size);
  }
  check(received==expected,"remaining SRT stream changed or stopped when another sender closed");
}
int wmain(int argc,wchar_t **argv) {
  if(argc!=2)return 2;
  const auto directory=fs::current_path()/(L"srt-interop-"+std::to_wstring(GetCurrentProcessId()));
  HMODULE library=nullptr;
  try {
    check(fs::create_directory(directory),"create isolated fixture");
    fs::copy_file(argv[1],directory/L"BonDriver_SRT.dll");
    library=LoadLibraryW((directory/L"BonDriver_SRT.dll").c_str());check(library!=nullptr,"load BonDriver");
    auto create=reinterpret_cast<IBonDriver *(*)()>(GetProcAddress(library,"CreateBonDriver"));check(create!=nullptr,"BonDriver entrypoint");
    {
      std::unique_ptr<IBonDriver2,DriverRelease> driver(static_cast<IBonDriver2*>(create()));
      std::unique_ptr<void,decltype(&udp_free)> sender(udp_new(0),udp_free);
      check(driver&&sender,"create sender and receiver");
      udp_reset(sender.get(),1);
      for(const char *secret:{"","integration-test-key"}) {
        {std::ofstream ini(directory/L"BonDriver_SRT.ini");ini<<"[SRT]\nListenAddress=127.0.0.1\nBasePort=33230\nLatencyMs=120\nPassphrase="<<secret<<'\n';}
        check(driver->OpenTuner()&&driver->SetChannel(0,0),"start listener");
        if(*secret) {
          for(const char *wrong:{"","incorrect-test-key"}) {
            check(stream_start_srt(sender.get(),token,"127.0.0.1",33230,0,120,wrong),"asynchronous start");
            auto deadline=GetTickCount64()+6000;RustStreamStatus status{};
            do {Sleep(20);udp_status(sender.get(),token,&status);}while(status.enabled&&GetTickCount64()<deadline);
            check(!status.enabled&&status.error&&!driver->GetReadyCount(),"reject missing/wrong key");
            udp_stop(sender.get(),token);
          }
        }
        transfer(*driver,sender.get(),secret);
        transfer(*driver,sender.get(),secret); // Reconnect to the same listener.
        driver->CloseTuner();
      }
      // Two source sessions in one DLL: stopping A must preserve B's runtime.
      {
        const char *secondToken="bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        auto *secondRaw=static_cast<IBonDriver2*>(create());
        check(secondRaw && secondRaw!=driver.get(),"independent receiver instances");
        std::unique_ptr<IBonDriver2,DriverRelease> second(secondRaw);
        check(driver->OpenTuner() && driver->SetChannel(0,0) && second->OpenTuner() && second->SetChannel(0,1),"open concurrent receiver ports");
        check(stream_start_srt(sender.get(),secondToken,"127.0.0.1",33231,6,120,"integration-test-key"),"start second sender");
        transfer(*driver,sender.get(),"integration-test-key");
        receiveExpected(*second,sample());
        auto packet=sample();packet.resize(188);
        check(udp_feed(sender.get(),packet.data(),packet.size()),"feed after first sender closes");
        receiveExpected(*second,packet);
        RustStreamStatus status{};
        check(udp_status(sender.get(),secondToken,&status) && status.enabled && status.bytes==sample().size()+188 && !status.error,"remaining sender status");
        check(udp_stop(sender.get(),secondToken),"stop second sender");
        second->CloseTuner();driver->CloseTuner();
      }
      check(stream_start_srt(sender.get(),token,"127.0.0.1",33230,0,120,""),"start without listener");
      auto stopped=GetTickCount64();udp_stop(sender.get(),token);
      check(GetTickCount64()-stopped<1000,"stop during connection");
    }
    FreeLibrary(library);library=nullptr;
    fs::remove(directory/L"BonDriver_SRT.dll");fs::remove(directory/L"BonDriver_SRT.ini");fs::remove(directory);
    puts("PASS: TvRockDTV -> BonDriver_SRT plaintext/AES-256 byte equality, reconnect, key rejection, partial TS input, counters, independent multi-session stop and bounded stop");return 0;
  } catch(const std::exception &e) {
    fprintf(stderr,"FAIL: %s\n",e.what());if(library)FreeLibrary(library);return 1;
  }
}



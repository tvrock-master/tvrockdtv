// Small private C ABI: the SRT reference implementation and crypto are static.
#include <srt.h>
#include <ws2tcpip.h>
#include <cstdint>
#include <cstring>
#include <mutex>

namespace {
std::mutex runtimeMutex;
size_t runtimeUsers = 0;
bool acquireRuntime() {
    std::lock_guard<std::mutex> guard(runtimeMutex);
    if (!runtimeUsers && srt_startup() == SRT_ERROR) return false;
    ++runtimeUsers;
    return true;
}
void releaseRuntime() {
    std::lock_guard<std::mutex> guard(runtimeMutex);
    if (runtimeUsers && --runtimeUsers == 0) srt_cleanup();
}
} // namespace


extern "C" int tr_srt_open(const uint8_t *ip, uint16_t port, int latency, const char *secret) {
  if(!acquireRuntime())return -1;
  auto sock=srt_create_socket();
  auto fail=[&](){if(sock!=SRT_INVALID_SOCK)srt_close(sock);releaseRuntime();return -1;};
  const SRT_TRANSTYPE live=SRTT_LIVE;
  const bool no=false, yes=true;
  const int key=32, payload=1316, timeout=3000;
  const linger immediate{0,0};
  if(sock==SRT_INVALID_SOCK ||
      srt_setsockflag(sock,SRTO_TRANSTYPE,&live,sizeof(live))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_SNDSYN,&no,sizeof(no))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_RCVSYN,&no,sizeof(no))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_SENDER,&yes,sizeof(yes))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_PEERLATENCY,&latency,sizeof(latency))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_PAYLOADSIZE,&payload,sizeof(payload))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_CONNTIMEO,&timeout,sizeof(timeout))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_LINGER,&immediate,sizeof(immediate))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_ENFORCEDENCRYPTION,&yes,sizeof(yes))==SRT_ERROR)return fail();
  if(secret[0] && (srt_setsockflag(sock,SRTO_PBKEYLEN,&key,sizeof(key))==SRT_ERROR ||
      srt_setsockflag(sock,SRTO_PASSPHRASE,secret,static_cast<int>(strlen(secret)))==SRT_ERROR))return fail();
  sockaddr_in address{};address.sin_family=AF_INET;address.sin_port=htons(port);
  memcpy(&address.sin_addr,ip,4);
  if(srt_connect(sock,reinterpret_cast<const sockaddr*>(&address),sizeof(address))==SRT_ERROR)return fail();
  return sock;
}
// 0 = connecting/backpressure, positive = accepted, -1 = disconnected.
extern "C" int tr_srt_send(int sock,const uint8_t *bytes,int size) {
  auto state=srt_getsockstate(sock);
  if(state==SRTS_CONNECTING || state==SRTS_OPENED)return 0;
  if(state!=SRTS_CONNECTED)return -1;
  int result=srt_sendmsg(sock,reinterpret_cast<const char*>(bytes),size,-1,1);
  if(result==SRT_ERROR && srt_getlasterror(nullptr)==SRT_EASYNCSND)return 0;
  return result;
}
extern "C" int tr_srt_state(int sock) {
  auto state=srt_getsockstate(sock);
  return state==SRTS_CONNECTED?1:(state==SRTS_CONNECTING || state==SRTS_OPENED?0:-1);
}
extern "C" void tr_srt_close(int sock) {srt_close(sock);releaseRuntime();}

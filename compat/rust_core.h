#pragma once
#include <stddef.h>
#include <stdint.h>
extern "C" {
struct RustStreamStatus {
  uint32_t enabled; char address[16]; uint16_t port,service; uint32_t flags;
  uint64_t bytes,lastTick; uint32_t error;
};
void *udp_new(uint32_t options);
void udp_free(void *);
bool stream_start_srt(const void *,const char *,const char *,uint16_t,uint32_t,uint32_t,const char *);
bool stream_is_srt(const void *,const char *);
bool udp_start(const void *,const char *session,const char *address,uint16_t port,uint32_t flags);
bool udp_stop(const void *,const char *session);
bool udp_pace(const void *,const char *session);
bool udp_is_paced(const void *,const char *session);
void udp_reset(const void *,uint16_t service);
void udp_service(const void *,uint16_t service);
bool udp_feed(const void *,const unsigned char *,size_t);
bool udp_status(const void *,const char *session,RustStreamStatus *);
void *tr_new();
void tr_free(void *);
void tr_reset(void *, uint32_t, uint16_t, uint32_t);
typedef bool (*RustEmit)(const char *);
bool tr_process(void *, const unsigned char *, size_t, RustEmit);
void tr_status(void *, uint32_t, float, uint32_t, uint32_t, uint32_t, RustEmit);
void tr_flush(void *, uint16_t, RustEmit, bool (*)(uint16_t));
void tr_resend(void *);
int64_t tr_time(const void *);
// Internal Rust client API. Shared ownership must outlive every call.
void *dtv_new(const char *device, const char *server, const char *parent);
void dtv_free(void *);
void dtv_stop(const void *);
bool dtv_wait_ready(const void *, uint32_t milliseconds);
uint32_t dtv_tuner(const void *);
bool dtv_valid(const void *, uint16_t);
uint64_t dtv_generation(const void *);
bool dtv_send(const void *, const char *utf8);
void dtv_reset_service(const void *);
bool dtv_send_wide(const void *, const wchar_t *, size_t length);
struct RustRemoteEvent {
  uint32_t kind, channel, service;
};
int32_t dtv_next(const void *, RustRemoteEvent *, wchar_t *path,
                 size_t capacity, uint32_t milliseconds);
struct RustFormat {
  uint32_t pos, size;
  unsigned char color, background, raster;
};
uint32_t tr_decode(wchar_t *, uint32_t, const unsigned char *, uint32_t,
                   uint32_t, void *, const wchar_t *(*)(void *, uint16_t),
                   void (*)(void *, const RustFormat *));
}

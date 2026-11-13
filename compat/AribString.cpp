// C++ API adapter; all decoding and bounds checks live in Rust.
#include "AribString.h"
#include "rust_core.h"
struct Context {
  CAribString::FormatList *formats;
  CAribString::IDRCSMap *map;
};
static const wchar_t *Map(void *p, uint16_t code) {
  auto &c = *static_cast<Context *>(p);
  return c.map ? c.map->GetString(code) : nullptr;
}
static void Format(void *p, const RustFormat *f) {
  auto &c = *static_cast<Context *>(p);
  if (c.formats)
    c.formats->push_back({f->pos, static_cast<CAribString::CHAR_SIZE>(f->size),
                          f->color, f->background, f->raster});
}
const DWORD CAribString::AribToString(TCHAR *dst, DWORD cap, const BYTE *src,
                                      DWORD len, unsigned flags) {
  return tr_decode(dst, cap, src, len, flags, nullptr, nullptr, nullptr);
}
const DWORD CAribString::CaptionToString(TCHAR *dst, DWORD cap, const BYTE *src,
                                         DWORD len, bool one,
                                         FormatList *formats, IDRCSMap *map) {
  if (formats)
    formats->clear();
  Context context{formats, map};
  return tr_decode(dst, cap, src, len,
                   FLAG_CAPTION | FLAG_USE_CHAR_SIZE | (one ? FLAG_1SEG : 0),
                   &context, Map, Format);
}

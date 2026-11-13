
#include "AribString.h"
#include <cassert>
#include <string>
#include <vector>
#include <cstdio>
#include <memory>
using Bytes=std::vector<BYTE>;
static std::wstring decode(const Bytes &s,unsigned flags=CAribString::FLAG_USE_CHAR_SIZE,DWORD capacity=256) {
    std::vector<WCHAR> out(capacity+2,0xabcd);
    DWORD n=CAribString::AribToString(out.data()+1,capacity,s.data(),static_cast<DWORD>(s.size()),flags);
    assert(out[0]==0xabcd && out[capacity+1]==0xabcd);
    assert(!capacity || (n<capacity && out[n+1]==0));
    return capacity ? std::wstring(out.data()+1,n) : L"";
}
struct Map:CAribString::IDRCSMap {
    LPCTSTR GetString(WORD code) override {return code==0x4121? L"DRCS" : nullptr;}
};
int main() {
    assert(decode({0x24,0x22})==L"\u3042"); // Kanji-set hiragana A.
    assert(decode({0xa2})==L"\u3042");
    assert(decode({0x0e,0x41})==L"\uff21");
    assert(decode({0x0e,0x89,0x41,0x5c,0x7e})==L"A\u00a5\u203e");
    assert(decode({0x0e,0x89,0x41,0x20},0)==L"\uff21\u3000");
    assert(decode({0x19,0x22,0x24,0x22})==L"\u3042\u3042");
    assert(decode({0x1b,0x24,0x28,0x42,0x24,0x22})==L"\u3042"); // Explicit G0 designation.
    assert(decode({0x1b,0x24,0x29,0x42,0x0e,0x24,0x22})==L"\u3042");
    assert(decode({0x1b,0x7e,0xc1})==L"\uff21");
    // Invalid second bytes must not consume the next shift/control.
    assert(decode({0x24,0x0e,0x41})==L"\uff21");
    assert(decode({0x1b,0x7c,0xa1})==L"\u30a1");
    assert(decode({0x24}).empty());
    assert(decode({0x98,0x43,0x20})==L"\u3000\u3000\u3000");
    WCHAR empty[4]={L'x',L'x',L'x',L'x'};
    assert(CAribString::AribToString(empty,4,nullptr,0)==0 && empty[0]==0);
    assert(CAribString::AribToString(nullptr,4,nullptr,0)==0);
    assert(decode({0x90,0x20}).empty()); // Truncated palette selection.
    assert(decode({0x9d,0x28,0x31}).empty()); // Missing TIME terminator.
    assert(decode({0x9d,0x28,0x31,0x40,0x0e,0x41})==L"\uff21");
    assert(decode({0x92,0x40,0x0e,0x41})==L"\uff21");
    assert(decode({0x92,0x20,0x40,0x0e,0x41})==L"\uff21");
    assert(decode({0x95,0x40,0x60,0x4f,0x41,0x95,0x4f,0x0e,0x42})==L"\uff22");
    assert(decode({0x95,0x4f,0x0e,0x42})==L"\uff22");
    assert(decode({0x9b,0x31,0x20,0x61,0x0e,0x41})==L"\uff21");
    const unsigned symbols=CAribString::FLAG_UNICODE_SYMBOL;
    assert(decode({0x7e,0x41},symbols)==L"\U0001f110");
    assert(decode({0x7e,0x49},symbols)==L"\U0001f118");
    assert(decode({0x98,0x43,0x7e,0x41},symbols)==L"\U0001f110\U0001f110\U0001f110");
    assert(decode({0x98,0x43,0x7e,0x41},symbols,4)==L"\U0001f110");
    assert(decode({0x7e,0x41},symbols,2).empty());
    assert(decode({0x7e,0x41},symbols,3)==L"\U0001f110");
    // Macro 0x6f changes GL to alphanumeric, even with only one output slot.
    WCHAR out[256]; Bytes macro={0x1d,0x6f,0x41};
    assert(CAribString::CaptionToString(out,2,macro.data(),3)==1 && out[0]==L'\uff21');
    CAribString::FormatList formats;
    Bytes colors={0x90,0x20,0x42,0x90,0x43,0x90,0x54,0x0e,0x41};
    CAribString::CaptionToString(out,256,colors.data(),static_cast<DWORD>(colors.size()),false,&formats);
    assert(formats.size()==1 && formats[0].CharColorIndex==0x23 && formats[0].BackColorIndex==0x24);
    CAribString::CaptionToString(out,256,macro.data(),3,false,&formats); assert(formats.empty());
    Map map;Bytes drcs={0x98,0x42,0x21};
    assert(CAribString::CaptionToString(out,256,drcs.data(),3,true,nullptr,&map)==8 && std::wstring(out)==L"DRCSDRCS");
    // Every truncated prefix with exact input/output allocations (ASan redzones).
    std::vector<Bytes> samples={colors,macro,drcs,{0x9d,0x28,0x31,0x3b,0x32,0x20,0x40},
        {0x1b,0x24,0x28,0x20,0x40,0x21,0x21},{0x9b,0x32,0x20,0x69,0x21,0x9b,0x30,0x20,0x69},
        {0x95,0x40,0x60,0x4f,0x95,0x4f},{0x1c,0x40,0x40},{0x16,0x41},
        {0x8b,0x60},{0x92,0x20,0x40},{0x91,0x40},{0x93,0x40},{0x94,0x40},{0x97,0x40}};
    for(const auto &sample:samples) for(size_t n=1;n<=sample.size();++n) {
        std::unique_ptr<BYTE[]> input(new BYTE[n]);memcpy(input.get(),sample.data(),n);
        for(DWORD cap=1;cap<=10;++cap) {
            std::unique_ptr<WCHAR[]> output(new WCHAR[cap]);
            DWORD size=CAribString::CaptionToString(output.get(),cap,input.get(),static_cast<DWORD>(n),false,&formats,&map);
            assert(size<cap && output[size]==0);
        }
    }
    unsigned int rng=0x12345678;
    for(int run=0;run<25000;++run) {
        rng=rng*1664525+1013904223;DWORD n=1+rng%128;
        std::unique_ptr<BYTE[]> input(new BYTE[n]);
        for(DWORD i=0;i<n;++i){rng=rng*1664525+1013904223;input[i]=rng>>24;}
        DWORD cap=1+((rng>>8)%32);std::unique_ptr<WCHAR[]> output(new WCHAR[cap]);
        DWORD size=CAribString::CaptionToString(output.get(),cap,input.get(),n,(run&1)!=0,&formats,&map);
        assert(size<cap && output[size]==0);
        size=CAribString::AribToString(output.get(),cap,input.get(),n,run&15);
        assert(size<cap && output[size]==0);
    }
    puts("ARIB string tests passed");
}

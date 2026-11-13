#pragma once

namespace TvRock {

#define NUM_DEVICE	8
#define STRING_MAX	1024

#define TVROCK_DTV_RECORD_START				(0x010)
#define TVROCK_DTV_RECORD_STOP				(0x011)
#define TVROCK_DTV_RECORD_START_QUICK		(0x012)
#define TVROCK_DTV_POWER_OFF				(0x013)
#define TVROCK_DTV_CHANNEL_CHANGE			(0x014)
#define TVROCK_DTV_SCREEN_CAPTURE			(0x015)
#define TVROCK_DTV_RECEIVE_TARGET			(0x016)
#define TVROCK_DTV_ON_OPTION				(0x017)
#define TVROCK_DTV_VIDEO_HWND				(0x018)
#define TVROCK_DTV_GET_EPG					(0x019)
#define TVROCK_DTV_FINISH_EPG				(0x01a)
#define TVROCK_DTV_CANCEL_EPG				(0x01b)

#define TVROCK_TUNER_TYPE_TS				(0x00)
#define TVROCK_TUNER_TYPE_BSCS				(0x01)

	struct RecordInfo {
		DWORD recMode;
		DWORD reserved;
		DWORD reserved2;
		DWORD reserved3;
		WCHAR fileName[STRING_MAX];
		WCHAR fileExt[16];
	};

	// デバイス種類
	enum {
		DEV_CAPTURE = 0,
		DEV_EXT_REC_1,
		DEV_EXT_REC_2,
		DEV_EXT_REC_3,
		DEV_EXT_REC_4,
		DEV_TUNER,
		DEV_TUNER_2,
		DEV_TUNER_3,
		DEV_TUNER_4,
		DEV_TUNER_5,
		DEV_TUNER_6,
		DEV_TUNER_7,
		DEV_TUNER_8,
		DEV_RADIO,
		NUM_TVROCK_DEVICE
	};

	enum {
		BC_VIEW_TS = 0x01,		// 地上波
		BC_VIEW_BS = 0x02,		// BS
		BC_VIEW_CS = 0x04,		// CS
		BC_VIEW_RD = 0x08,		// Radio
	};

#ifdef	_USRDLL
#define	_DLL_EXPORT extern "C" __declspec(dllexport)
#else
#define	_DLL_EXPORT
#endif

#define _MY_MALLOC(x)	HeapAlloc(GetProcessHeap(), NULL, (x));
#define _MY_FREE(x)		HeapFree(GetProcessHeap(), NULL, (x));

	extern DWORD	dwDeviceID;
	extern DWORD	dwCh;
	extern DWORD	dwServiceID;

	extern void		trace(LPWSTR lpszFormat, ...);
	extern void		trace2(LPCSTR lpszFormat, ...);

	extern BOOL		ReadSoftwareRegistry(const LPWSTR key, const LPWSTR name, DWORD *val);
	extern BOOL		ReadSoftwareRegistry(const LPWSTR key, const LPWSTR name, LPWSTR buf, const DWORD maxsz);

	extern bool CheckDupeFile(const DWORD devNo, WCHAR *fname, WCHAR *ext, WCHAR newname[]);

}

"""Import the original character mapping data (no C++ code is compiled)."""
import re, json, pathlib, sys
source = pathlib.Path(sys.argv[1]).read_text(encoding='utf-8-sig')
out = pathlib.Path(__file__).resolve().parents[1] / 'src' / 'tables.rs'
def literal(s):
    s = re.sub(r'\\[uU]([0-9a-fA-F]{4,8})', lambda m: chr(int(m[1],16)), s)
    return json.dumps(s, ensure_ascii=False)
lines=['// Character mappings imported from the original AribString.cpp.']
for name, body in re.findall(r'static const LPC(?:TSTR|WSTR) (\w+)\[\]\s*=\s*\{(.*?)\};', source, re.S):
    body=re.sub(r'//[^\n]*','',body)
    vals=re.findall(r'(?:L|_T\()"((?:\\.|[^"\\])*)"\)?|(nullptr)',body)
    lines.append('pub const '+name.upper()+': &[&str] = &['+','.join('"□"' if null else literal(s) for s,null in vals)+'];')
for name, body in re.findall(r'static const LPCTSTR (ac\w+Table)\s*=\s*(.*?);',source,re.S):
    vals=re.findall(r'TEXT\("(.*?)"\)',body)
    if vals and name != 'acAlphanumericTable': lines.append('pub const '+name.upper()+': &str = '+literal(''.join(vals))+';')
body=re.search(r'static const BYTE Macro\[16\]\[20\] = \{(.*?)\n\s*\};',source,re.S)[1]
rows=re.findall(r'\{(.*?)\}',body,re.S)
lines.append('pub const MACROS: &[&[u8]] = &['+','.join('&['+row+']' for row in rows)+'];')
out.write_text('\n'.join(lines)+'\n',encoding='utf-8')

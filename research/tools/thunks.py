# Usage: thunks.py EXE [VA...] -> name the xlive import (by ordinal) behind each `jmp [IAT]` thunk / call site
import sys, pefile, struct, re
pe = pefile.PE(sys.argv[1]); base = pe.OPTIONAL_HEADER.ImageBase
names = dict(l.split() for l in open(sys.argv[2])) if len(sys.argv) > 2 and not sys.argv[2].startswith("0x") else {}
iat = {}
for d in pe.DIRECTORY_ENTRY_IMPORT:
    for imp in d.imports:
        iat[imp.address] = f"{d.dll.decode()}!{imp.name.decode() if imp.name else names.get(str(imp.ordinal), '#'+str(imp.ordinal))}"
text = next(s for s in pe.sections if s.Name.startswith(b".text")); code = text.get_data(); tva = base + text.VirtualAddress
for va in [int(a, 16) for a in sys.argv[2:] if a.startswith("0x")] or []:
    o = va - tva; b = code[o:o + 6]
    tgt = struct.unpack_from("<I", b, 2)[0] if b[:2] == b"\xff\x25" else None
    print(hex(va), iat.get(tgt, f"not a thunk: {b.hex()}"))

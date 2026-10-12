# Usage: props.py EXE INSERT_VA classes.json OUT.json
# For every function that calls MtPropertyList::insert(INSERT_VA): recover each MtProperty
# {name, type, attr, field offset from the owner} and the class it belongs to (via getDTI in its vtable).
import sys, pefile, struct, json, re
from capstone import Cs, CS_ARCH_X86, CS_MODE_32
from capstone.x86 import X86_OP_IMM, X86_OP_REG, X86_OP_MEM
exe, insert, cj, out = sys.argv[1], int(sys.argv[2], 16), sys.argv[3], sys.argv[4]
pe = pefile.PE(exe, fast_load=True); base = pe.OPTIONAL_HEADER.ImageBase
text = next(s for s in pe.sections if s.Name.startswith(b".text")); code = text.get_data(); tva = base + text.VirtualAddress
rdata = next(s for s in pe.sections if s.Name.startswith(b".rdata")); rd = rdata.get_data(); rva0 = base + rdata.VirtualAddress
dti = {c["dti"]: c["name"] for c in json.load(open(cj))}
md = Cs(CS_ARCH_X86, CS_MODE_32); md.detail = True
def cstr(va):
    try: s = pe.get_data(va - base, 80).split(b"\0")[0]
    except Exception: return None
    return s.decode() if s and all(32 <= c < 127 for c in s) else None
def func_start(va):
    o = va - tva
    while o > 0 and not (code[o - 1] == 0xCC and code[o] != 0xCC): o -= 1
    return tva + o
calls = set()
for i in range(len(code) - 5):
    if code[i] == 0xE8 and (tva + i + 5 + struct.unpack_from("<i", code, i + 1)[0]) & 0xFFFFFFFF == insert:
        calls.add(func_start(tva + i))
# getDTI implementations: `mov eax, imm32; ret`
getdti = {}
for m in re.finditer(rb"\xB8(....)\xC3", code):
    v = struct.unpack("<I", m.group(1))[0]
    if v in dti: getdti[tva + m.start()] = dti[v]
def owner_class(fn):
    for m in re.finditer(re.escape(struct.pack("<I", fn)), rd):
        slot = rva0 + m.start()
        for k in range(-12, 13):  # look around the slot for this vtable's getDTI
            o = m.start() + 4 * k
            if 0 <= o <= len(rd) - 4:
                p = struct.unpack_from("<I", rd, o)[0]
                if p in getdti: return getdti[p]
    return None
result = {}
for fn in sorted(calls):
    regs = {}; esp = 0; stack = {}; fields = []
    body = code[fn - tva: fn - tva + 0x6000]
    for ins in md.disasm(body, fn):
        mn, ops = ins.mnemonic, ins.operands
        def val(op):
            if op.type == X86_OP_IMM: return ("imm", op.imm & 0xFFFFFFFF)
            if op.type == X86_OP_REG: return regs.get(ins.reg_name(op.reg))
            return None
        if mn == "push":
            esp -= 4; stack[esp] = val(ops[0])
        elif mn == "pop": esp += 4; regs.pop(ins.reg_name(ops[0].reg), None) if ops[0].type == X86_OP_REG else None
        elif mn == "sub" and ops[0].type == X86_OP_REG and ins.reg_name(ops[0].reg) == "esp" and ops[1].type == X86_OP_IMM: esp -= ops[1].imm
        elif mn == "add" and ops[0].type == X86_OP_REG and ins.reg_name(ops[0].reg) == "esp" and ops[1].type == X86_OP_IMM: esp += ops[1].imm
        elif mn == "mov" and ops[0].type == X86_OP_REG:
            r = ins.reg_name(ops[0].reg)
            if ops[1].type == X86_OP_MEM and ops[1].mem.base and ins.reg_name(ops[1].mem.base) == "esp":
                regs[r] = stack.get(esp + ops[1].mem.disp)
            else: regs[r] = val(ops[1])
            if r in ("ecx", "cx"): regs["cx"] = regs.get(r)
        elif mn == "lea" and ops[0].type == X86_OP_REG:
            m_ = ops[1].mem; b = ins.reg_name(m_.base) if m_.base else None
            r = ins.reg_name(ops[0].reg)
            if b == "esp": regs[r] = ("stk", esp + m_.disp)
            elif b in ("esi", "edi", "ebx", "ebp") and not m_.index: regs[r] = ("fld", b, m_.disp)
            else: regs[r] = None
        elif mn == "mov" and ops[0].type == X86_OP_MEM and ops[0].mem.base and ins.reg_name(ops[0].mem.base) == "esp":
            v = val(ops[1])
            if ops[1].type == X86_OP_REG and ins.reg_name(ops[1].reg) in ("cx", "dx", "ax"):
                v = regs.get({"cx": "ecx", "dx": "edx", "ax": "eax"}[ins.reg_name(ops[1].reg)])
            stack[esp + ops[0].mem.disp] = v
        elif mn == "xor" and ops[0].type == X86_OP_REG and ops[1].type == X86_OP_REG and ops[0].reg == ops[1].reg:
            regs[ins.reg_name(ops[0].reg)] = ("imm", 0)
        elif mn == "call":
            if ops[0].type == X86_OP_IMM and ops[0].imm == insert:
                arg = stack.get(esp)
                if arg and arg[0] == "stk":
                    b = arg[1]; g = lambda k: stack.get(b + k)
                    name = g(0); typ = g(4); addr = g(0xc)
                    n = cstr(name[1]) if name and name[0] == "imm" else None
                    t = typ[1] & 0xFFFF if typ and typ[0] == "imm" else None
                    off = addr[2] if addr and addr[0] == "fld" else None
                    if n: fields.append(dict(name=n, type=t, offset=off))
            for r in ("eax", "ecx", "edx"): regs.pop(r, None)
        elif mn in ("ret",) or (mn == "jmp" and ops[0].type == X86_OP_REG): break
    if fields:
        cls = owner_class(fn) or f"fn_{fn:x}"
        result.setdefault(cls, []).extend(fields)
json.dump(result, open(out, "w"), indent=0)
print(len(calls), "createProperty functions,", len(result), "classes,", sum(map(len, result.values())), "fields")

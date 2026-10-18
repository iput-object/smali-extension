#!/usr/bin/env python3
"""Generate src/table.rs from google/smali's dexlib2 Opcode.java.

Usage: gen_opcodes.py <path/to/Opcode.java> > crates/smali-opcodes/src/table.rs

Sources:
- Opcode values, names, formats, reference types and flags: dexlib2 Opcode.java.
- Operand layout per format: smaliParser.g `insn_format*` rules
  (https://github.com/google/smali/blob/main/smali/src/main/antlr/smaliParser.g)
  and AOSP "Dalvik executable instruction formats"
  (https://source.android.com/docs/core/runtime/instruction-formats).
- Wide operands: AOSP "Dalvik bytecode"
  (https://source.android.com/docs/core/runtime/dalvik-bytecode).
"""

import re
import sys

# Operand layouts. R4/R8/R16: register with that many bits. LIST: {..} non-range
# (4-bit regs, max 5). RANGE: {vA .. vB}. LABEL. LITn: integral literal of n bits.
# HIGH32/HIGH64: const/high16 literals. LIT32/LIT64 for 31i/51l. REF: the opcode's
# reference type. PROTO: method prototype. ODEX: odex-only payload.
FORMATS = {
    "Format10t": ["LABEL"],
    "Format10x": [],
    "Format11n": ["R4", "LIT4"],
    "Format11x": ["R8"],
    "Format12x": ["R4", "R4"],
    "Format20bc": ["ODEX", "ODEX"],
    "Format20t": ["LABEL"],
    "Format21c": ["R8", "REF"],
    "Format21ih": ["R8", "HIGH32"],
    "Format21lh": ["R8", "HIGH64"],
    "Format21s": ["R8", "LIT16"],
    "Format21t": ["R8", "LABEL"],
    "Format22b": ["R8", "R8", "LIT8"],
    "Format22c": ["R4", "R4", "REF"],
    "Format22cs": ["R4", "R4", "ODEX"],
    "Format22s": ["R4", "R4", "LIT16"],
    "Format22t": ["R4", "R4", "LABEL"],
    "Format22x": ["R8", "R16"],
    "Format23x": ["R8", "R8", "R8"],
    "Format30t": ["LABEL"],
    "Format31c": ["R8", "REF"],
    "Format31i": ["R8", "LIT32"],
    "Format31t": ["R8", "LABEL"],
    "Format32x": ["R16", "R16"],
    "Format35c": ["LIST", "REF"],
    "Format35mi": ["LIST", "ODEX"],
    "Format35ms": ["LIST", "ODEX"],
    "Format3rc": ["RANGE", "REF"],
    "Format3rmi": ["RANGE", "ODEX"],
    "Format3rms": ["RANGE", "ODEX"],
    "Format45cc": ["LIST", "REF", "PROTO"],
    "Format4rcc": ["RANGE", "REF", "PROTO"],
    "Format51l": ["R8", "LIT64"],
}

KIND = {
    "R4": "OperandKind::Reg(4)", "R8": "OperandKind::Reg(8)", "R16": "OperandKind::Reg(16)",
    "LIST": "OperandKind::RegList", "RANGE": "OperandKind::RegRange", "LABEL": "OperandKind::Label",
    "LIT4": "OperandKind::Lit(LitKind::Bits4)", "LIT8": "OperandKind::Lit(LitKind::Bits8)",
    "LIT16": "OperandKind::Lit(LitKind::Bits16)", "LIT32": "OperandKind::Lit(LitKind::Bits32)",
    "LIT64": "OperandKind::Lit(LitKind::Bits64)", "HIGH32": "OperandKind::Lit(LitKind::High16)",
    "HIGH64": "OperandKind::Lit(LitKind::WideHigh16)", "PROTO": "OperandKind::Proto",
    "ODEX": "OperandKind::Odex",
}

REFS = {
    "NONE": "RefKind::None", "STRING": "RefKind::String", "TYPE": "RefKind::Type",
    "FIELD": "RefKind::Field", "METHOD": "RefKind::Method", "CALL_SITE": "RefKind::CallSite",
    "METHOD_PROTO": "RefKind::MethodProto", "METHOD_HANDLE": "RefKind::MethodHandle",
}

FLAGS = ["CAN_THROW", "ODEX_ONLY", "CAN_CONTINUE", "SETS_RESULT", "SETS_REGISTER",
         "SETS_WIDE_REGISTER", "QUICK_FIELD_ACCESSOR", "VOLATILE_FIELD_ACCESSOR",
         "STATIC_FIELD_ACCESSOR", "JUMBO_OPCODE", "CAN_INITIALIZE_REFERENCE"]

ENTRY = re.compile(
    r'^\s+([A-Z0-9_]+)\((.*?),\s*"([^"]+)",\s*ReferenceType\.(\w+),\s*(?:ReferenceType\.\w+,\s*)?Format\.(\w+)(?:,\s*([^)]*))?\)[,;]?\s*$')
VALUE = re.compile(r"0x[0-9a-fA-F]+")

SHIFTS = ("shl-", "shr-", "ushr-")
WIDE_BINOPS = ("add-", "sub-", "mul-", "div-", "rem-", "and-", "or-", "xor-")


def wide_mask(name, flags, n_ops):
    """Bit i set when register operand i is the low half of a wide pair."""
    mask = 0
    if "SETS_WIDE_REGISTER" in flags:
        mask |= 1
    base = name.split("/")[0]
    if base in ("return-wide",) or base.startswith(("aput-wide", "iput-wide", "sput-wide")):
        mask |= 1
    if base.startswith("move-wide"):
        mask |= 0b11
    if base in ("neg-long", "not-long", "neg-double") or base.startswith(("long-to-", "double-to-")):
        mask |= 0b10
    if base in ("cmp-long", "cmpl-double", "cmpg-double"):
        mask |= 0b110
    if base.endswith(("-long", "-double")) and base.startswith(WIDE_BINOPS + SHIFTS):
        two_addr = name.endswith("/2addr")
        if base.startswith(SHIFTS):
            mask |= 0b1 if two_addr else 0b10
        else:
            mask |= 0b11 if two_addr else 0b110
    return mask & ((1 << n_ops) - 1)


def main():
    src = open(sys.argv[1]).read()
    seen = set()
    rows = []
    for line in src.splitlines():
        m = ENTRY.match(line)
        if not m:
            continue
        _, versions, name, ref, fmt, flagexpr = m.groups()
        if fmt not in FORMATS or name in seen:
            continue
        seen.add(name)
        flags = [f for f in FLAGS if f"Opcode.{f}" in (flagexpr or "")]
        value = int(VALUE.search(versions).group(0), 16)
        ops = FORMATS[fmt]
        n_regs = sum(1 for o in ops if o in ("R4", "R8", "R16"))
        kinds = [KIND.get(o, f"OperandKind::Ref({REFS[ref]})") for o in ops]
        flag_bits = " | ".join(f"flags::{f}" for f in flags) or "0"
        fmt_id = fmt.replace("Format", "F")
        rows.append(
            f'    Opcode {{ name: "{name}", value: 0x{value:02x}, format: Format::{fmt_id}, '
            f'reference: {REFS[ref]}, flags: {flag_bits}, operands: &[{", ".join(kinds)}], '
            f'wide_mask: 0b{wide_mask(name, flags, n_regs):b} }},')
    print("// @generated by gen/gen_opcodes.py from google/smali dexlib2 Opcode.java. Do not edit.")
    print("use crate::*;\n")
    print("pub static OPCODES: &[Opcode] = &[")
    print("\n".join(rows))
    print("];")


if __name__ == "__main__":
    main()

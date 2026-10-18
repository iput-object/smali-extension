//! Dalvik opcode table, generated from google/smali's `Opcode.java` by `gen/gen_opcodes.py`.

mod table;

use std::collections::HashMap;
use std::sync::OnceLock;

pub use table::OPCODES;

/// Instruction format ids from AOSP "Dalvik executable instruction formats".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    F10t,
    F10x,
    F11n,
    F11x,
    F12x,
    F20bc,
    F20t,
    F21c,
    F21ih,
    F21lh,
    F21s,
    F21t,
    F22b,
    F22c,
    F22cs,
    F22s,
    F22t,
    F22x,
    F23x,
    F30t,
    F31c,
    F31i,
    F31t,
    F32x,
    F35c,
    F35mi,
    F35ms,
    F3rc,
    F3rmi,
    F3rms,
    F45cc,
    F4rcc,
    F51l,
}

impl Format {
    /// Format id as written in the AOSP docs, e.g. `35c`.
    pub fn id(self) -> &'static str {
        match self {
            Format::F10t => "10t",
            Format::F10x => "10x",
            Format::F11n => "11n",
            Format::F11x => "11x",
            Format::F12x => "12x",
            Format::F20bc => "20bc",
            Format::F20t => "20t",
            Format::F21c => "21c",
            Format::F21ih => "21ih",
            Format::F21lh => "21lh",
            Format::F21s => "21s",
            Format::F21t => "21t",
            Format::F22b => "22b",
            Format::F22c => "22c",
            Format::F22cs => "22cs",
            Format::F22s => "22s",
            Format::F22t => "22t",
            Format::F22x => "22x",
            Format::F23x => "23x",
            Format::F30t => "30t",
            Format::F31c => "31c",
            Format::F31i => "31i",
            Format::F31t => "31t",
            Format::F32x => "32x",
            Format::F35c => "35c",
            Format::F35mi => "35mi",
            Format::F35ms => "35ms",
            Format::F3rc => "3rc",
            Format::F3rmi => "3rmi",
            Format::F3rms => "3rms",
            Format::F45cc => "45cc",
            Format::F4rcc => "4rcc",
            Format::F51l => "51l",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    None,
    String,
    Type,
    Field,
    Method,
    CallSite,
    MethodProto,
    MethodHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LitKind {
    Bits4,
    Bits8,
    Bits16,
    Bits32,
    Bits64,
    /// `const/high16`: 32-bit value with the low 16 bits zero.
    High16,
    /// `const-wide/high16`: 64-bit value with the low 48 bits zero.
    WideHigh16,
}

impl LitKind {
    /// Signed range accepted for this literal.
    pub fn range(self) -> (i128, i128) {
        match self {
            LitKind::Bits4 => (-8, 7),
            LitKind::Bits8 => (-128, 127),
            LitKind::Bits16 => (-32768, 32767),
            LitKind::Bits32 | LitKind::High16 => (i32::MIN as i128, i32::MAX as i128),
            LitKind::Bits64 | LitKind::WideHigh16 => (i64::MIN as i128, i64::MAX as i128),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperandKind {
    /// A single register with the given operand width in bits.
    Reg(u8),
    /// `{v0, v1}`: up to 5 registers, each 4-bit.
    RegList,
    /// `{v0 .. v5}`.
    RegRange,
    Label,
    Lit(LitKind),
    Ref(RefKind),
    Proto,
    /// Odex-only payload (vtable/inline index, field offset, verification error).
    Odex,
}

pub mod flags {
    pub const CAN_THROW: u32 = 0x1;
    pub const ODEX_ONLY: u32 = 0x2;
    pub const CAN_CONTINUE: u32 = 0x4;
    pub const SETS_RESULT: u32 = 0x8;
    pub const SETS_REGISTER: u32 = 0x10;
    pub const SETS_WIDE_REGISTER: u32 = 0x20;
    pub const QUICK_FIELD_ACCESSOR: u32 = 0x40;
    pub const VOLATILE_FIELD_ACCESSOR: u32 = 0x80;
    pub const STATIC_FIELD_ACCESSOR: u32 = 0x100;
    pub const JUMBO_OPCODE: u32 = 0x200;
    pub const CAN_INITIALIZE_REFERENCE: u32 = 0x400;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opcode {
    pub name: &'static str,
    pub value: u16,
    pub format: Format,
    pub reference: RefKind,
    pub flags: u32,
    pub operands: &'static [OperandKind],
    /// Bit i set when the i-th single-register operand is the low half of a wide pair.
    pub wide_mask: u8,
}

impl Opcode {
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    /// Odex-only opcodes are rejected by the `smali` assembler (`throwOdexedInstructionException`).
    pub fn is_odex_only(&self) -> bool {
        self.has(flags::ODEX_ONLY)
    }

    pub fn is_invoke(&self) -> bool {
        self.name.starts_with("invoke-")
    }

    pub fn is_range(&self) -> bool {
        self.operands.contains(&OperandKind::RegRange)
    }

    pub fn is_branch(&self) -> bool {
        self.operands.contains(&OperandKind::Label) && !matches!(self.format, Format::F31t)
    }

    pub fn is_return(&self) -> bool {
        self.name.starts_with("return")
    }

    /// Instructions after which `move-result*` is allowed.
    pub fn sets_result(&self) -> bool {
        self.has(flags::SETS_RESULT)
    }

    pub fn is_move_result(&self) -> bool {
        self.name.starts_with("move-result")
    }

    pub fn is_wide_reg(&self, reg_index: usize) -> bool {
        reg_index < 8 && self.wide_mask & (1 << reg_index) != 0
    }

    /// One-line description for hover.
    pub fn summary(&self) -> String {
        let ops: Vec<&str> = self.operands.iter().map(|o| operand_label(*o)).collect();
        format!("{} {}", self.name, ops.join(", "))
    }
}

fn operand_label(kind: OperandKind) -> &'static str {
    match kind {
        OperandKind::Reg(4) => "vA (4-bit)",
        OperandKind::Reg(8) => "vAA (8-bit)",
        OperandKind::Reg(_) => "vAAAA (16-bit)",
        OperandKind::RegList => "{vC, vD, vE, vF, vG}",
        OperandKind::RegRange => "{vCCCC .. vNNNN}",
        OperandKind::Label => ":label",
        OperandKind::Lit(LitKind::Bits4) => "#lit4",
        OperandKind::Lit(LitKind::Bits8) => "#lit8",
        OperandKind::Lit(LitKind::Bits16) => "#lit16",
        OperandKind::Lit(LitKind::Bits32) => "#lit32",
        OperandKind::Lit(LitKind::Bits64) => "#lit64",
        OperandKind::Lit(LitKind::High16) => "#lit32 (low 16 bits zero)",
        OperandKind::Lit(LitKind::WideHigh16) => "#lit64 (low 48 bits zero)",
        OperandKind::Ref(RefKind::String) => "\"string\"",
        OperandKind::Ref(RefKind::Type) => "Ltype;",
        OperandKind::Ref(RefKind::Field) => "Lcls;->field:Ltype;",
        OperandKind::Ref(RefKind::Method) => "Lcls;->method()V",
        OperandKind::Ref(RefKind::CallSite) => "call_site",
        OperandKind::Ref(RefKind::MethodProto) => "(proto)V",
        OperandKind::Ref(RefKind::MethodHandle) => "method-handle",
        OperandKind::Ref(RefKind::None) => "?",
        OperandKind::Proto => "(proto)V",
        OperandKind::Odex => "odex",
    }
}

fn by_name() -> &'static HashMap<&'static str, &'static Opcode> {
    static MAP: OnceLock<HashMap<&'static str, &'static Opcode>> = OnceLock::new();
    MAP.get_or_init(|| OPCODES.iter().map(|o| (o.name, o)).collect())
}

pub fn lookup(name: &str) -> Option<&'static Opcode> {
    by_name().get(name).copied()
}

#[cfg(test)]
mod tests;

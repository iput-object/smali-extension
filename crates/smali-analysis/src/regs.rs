//! Register accounting for a method.

use smali_syntax::descriptor::{parse_proto, Proto};
use smali_syntax::model::{Method, Reg, RegDirectiveKind, RegKind};

#[derive(Debug, Clone)]
pub struct Frame {
    /// Parameter registers, including `this`.
    pub params: u32,
    /// Total registers, when exactly one register directive is present.
    pub total: Option<u64>,
    pub proto: Option<Proto>,
    pub is_static: bool,
}

impl Frame {
    pub fn of(method: &Method) -> Frame {
        let proto = parse_proto(&method.proto.text).ok();
        let is_static = method.is_static();
        let params = u32::from(!is_static) + proto.as_ref().map_or(0, Proto::param_slots);
        let total = match method.reg_directives.as_slice() {
            [d] => d.value.map(|v| match d.kind {
                RegDirectiveKind::Registers => v,
                RegDirectiveKind::Locals => v + u64::from(params),
            }),
            _ => None,
        };
        Frame {
            params,
            total,
            proto,
            is_static,
        }
    }

    pub fn locals(&self) -> Option<u64> {
        self.total
            .and_then(|t| t.checked_sub(u64::from(self.params)))
    }

    /// The `v` number a register denotes. `pN` needs the local count.
    pub fn v_number(&self, reg: &Reg) -> Option<u64> {
        match reg.kind {
            RegKind::V => Some(u64::from(reg.num)),
            RegKind::P => self.locals().map(|l| l + u64::from(reg.num)),
        }
    }

    /// Parameter described by `pN`: (index into params or None for `this`, type).
    pub fn param_of(&self, p: u32) -> Option<(Option<usize>, String)> {
        let mut slot = 0u32;
        if !self.is_static {
            if p == 0 {
                return Some((None, "this".to_string()));
            }
            slot = 1;
        }
        for (i, t) in self.proto.as_ref()?.params.iter().enumerate() {
            if p == slot || (t.is_wide() && p == slot + 1) {
                return Some((Some(i), t.to_string()));
            }
            slot += t.slots();
        }
        None
    }
}

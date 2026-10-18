use super::*;

/// 30 opcodes checked by hand against dexlib2 Opcode.java (value, format, reference).
#[test]
fn sample_matches_source() {
    let sample: &[(&str, u16, Format, RefKind)] = &[
        ("nop", 0x00, Format::F10x, RefKind::None),
        ("move", 0x01, Format::F12x, RefKind::None),
        ("move/from16", 0x02, Format::F22x, RefKind::None),
        ("move/16", 0x03, Format::F32x, RefKind::None),
        ("move-result-object", 0x0c, Format::F11x, RefKind::None),
        ("return-void", 0x0e, Format::F10x, RefKind::None),
        ("const/4", 0x12, Format::F11n, RefKind::None),
        ("const/16", 0x13, Format::F21s, RefKind::None),
        ("const", 0x14, Format::F31i, RefKind::None),
        ("const/high16", 0x15, Format::F21ih, RefKind::None),
        ("const-wide/high16", 0x19, Format::F21lh, RefKind::None),
        ("const-wide", 0x18, Format::F51l, RefKind::None),
        ("const-string", 0x1a, Format::F21c, RefKind::String),
        ("const-string/jumbo", 0x1b, Format::F31c, RefKind::String),
        ("const-class", 0x1c, Format::F21c, RefKind::Type),
        ("check-cast", 0x1f, Format::F21c, RefKind::Type),
        ("instance-of", 0x20, Format::F22c, RefKind::Type),
        ("filled-new-array", 0x24, Format::F35c, RefKind::Type),
        ("filled-new-array/range", 0x25, Format::F3rc, RefKind::Type),
        ("fill-array-data", 0x26, Format::F31t, RefKind::None),
        ("goto", 0x28, Format::F10t, RefKind::None),
        ("goto/32", 0x2a, Format::F30t, RefKind::None),
        ("sparse-switch", 0x2c, Format::F31t, RefKind::None),
        ("if-eq", 0x32, Format::F22t, RefKind::None),
        ("if-nez", 0x39, Format::F21t, RefKind::None),
        ("iget-object", 0x54, Format::F22c, RefKind::Field),
        ("sput-wide", 0x68, Format::F21c, RefKind::Field),
        ("invoke-virtual", 0x6e, Format::F35c, RefKind::Method),
        ("invoke-static/range", 0x77, Format::F3rc, RefKind::Method),
        ("add-int/lit8", 0xd8, Format::F22b, RefKind::None),
        ("invoke-polymorphic", 0xfa, Format::F45cc, RefKind::Method),
        (
            "const-method-type",
            0xff,
            Format::F21c,
            RefKind::MethodProto,
        ),
    ];
    for (name, value, format, reference) in sample {
        let op = lookup(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(op.value, *value, "{name}");
        assert_eq!(op.format, *format, "{name}");
        assert_eq!(op.reference, *reference, "{name}");
    }
}

#[test]
fn wide_operands() {
    let wide = |n: &str, i| lookup(n).map(|o| o.is_wide_reg(i)).unwrap_or(false);
    assert!(wide("move-wide", 0) && wide("move-wide", 1));
    assert!(wide("add-long", 0) && wide("add-long", 1) && wide("add-long", 2));
    assert!(wide("shl-long", 1) && !wide("shl-long", 2));
    assert!(wide("shl-long/2addr", 0) && !wide("shl-long/2addr", 1));
    assert!(!wide("cmp-long", 0) && wide("cmp-long", 1));
    assert!(!wide("long-to-int", 0) && wide("long-to-int", 1));
    assert!(wide("int-to-long", 0) && !wide("int-to-long", 1));
    assert!(wide("return-wide", 0) && wide("aput-wide", 0));
    assert!(!wide("add-int", 0));
}

#[test]
fn odex_and_flags() {
    assert!(lookup("iget-quick").is_some_and(|o| o.is_odex_only()));
    assert!(lookup("invoke-virtual").is_some_and(|o| o.sets_result() && o.is_invoke()));
    assert!(lookup("invoke-virtual/range").is_some_and(|o| o.is_range()));
    assert!(lookup("filled-new-array").is_some_and(|o| o.sets_result()));
    assert!(lookup("bogus").is_none());
}

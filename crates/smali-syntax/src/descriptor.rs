//! Type descriptors, method prototypes and member references.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeDesc {
    Prim(char),
    /// Binary name without the `L`/`;`, e.g. `java/lang/String`.
    Class(String),
    Array(u8, Box<TypeDesc>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescError {
    /// Byte offset of the problem relative to the start of the parsed text.
    pub offset: usize,
    pub len: usize,
    pub message: String,
}

impl DescError {
    fn new(offset: usize, len: usize, message: impl Into<String>) -> Self {
        DescError {
            offset,
            len: len.max(1),
            message: message.into(),
        }
    }
}

impl TypeDesc {
    /// Register slots: 2 for `J`/`D`, 0 for `V`, 1 otherwise.
    pub fn slots(&self) -> u32 {
        match self {
            TypeDesc::Prim('J' | 'D') => 2,
            TypeDesc::Prim('V') => 0,
            _ => 1,
        }
    }

    pub fn is_wide(&self) -> bool {
        self.slots() == 2
    }

    pub fn is_reference(&self) -> bool {
        matches!(self, TypeDesc::Class(_) | TypeDesc::Array(..))
    }

    /// Innermost class name for arrays of objects.
    pub fn class_name(&self) -> Option<&str> {
        match self {
            TypeDesc::Class(c) => Some(c),
            TypeDesc::Array(_, t) => t.class_name(),
            TypeDesc::Prim(_) => None,
        }
    }

    pub fn descriptor(&self) -> String {
        self.to_string()
    }

    /// Java-style rendering. `short` drops the package.
    pub fn java(&self, short: bool) -> String {
        match self {
            TypeDesc::Prim(c) => match c {
                'V' => "void",
                'Z' => "boolean",
                'B' => "byte",
                'S' => "short",
                'C' => "char",
                'I' => "int",
                'J' => "long",
                'F' => "float",
                'D' => "double",
                _ => "?",
            }
            .to_string(),
            TypeDesc::Class(c) => {
                let name = if short {
                    c.rsplit('/').next().unwrap_or(c)
                } else {
                    c.as_str()
                };
                name.replace(['/', '$'], ".")
            }
            TypeDesc::Array(n, t) => format!("{}{}", t.java(short), "[]".repeat(*n as usize)),
        }
    }
}

impl fmt::Display for TypeDesc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeDesc::Prim(c) => write!(f, "{c}"),
            TypeDesc::Class(c) => write!(f, "L{c};"),
            TypeDesc::Array(n, t) => write!(f, "{}{t}", "[".repeat(*n as usize)),
        }
    }
}

fn valid_simple_name_char(c: char) -> bool {
    // dex "SimpleName": letters, digits, $ - _ and anything outside ASCII (dex allows
    // most non-ASCII code points; obfuscators use them).
    !matches!(
        c,
        ';' | '/' | '[' | '(' | ')' | '.' | '<' | '>' | ':' | ' ' | '\t' | '\n' | '\r' | ','
    )
}

/// Parse one type at the start of `s`. Returns the type and the number of bytes consumed.
pub fn parse_type_prefix(s: &str, allow_void: bool) -> Result<(TypeDesc, usize), DescError> {
    let bytes = s.as_bytes();
    let mut dims = 0usize;
    while bytes.get(dims) == Some(&b'[') {
        dims += 1;
    }
    if dims > 255 {
        return Err(DescError::new(
            0,
            dims,
            "array has more than 255 dimensions",
        ));
    }
    let rest = &s[dims..];
    let (base, used) = match rest.chars().next() {
        None => return Err(DescError::new(dims, 0, "expected a type")),
        Some('V') if dims > 0 => {
            return Err(DescError::new(dims, 1, "array of void is not a valid type"))
        }
        Some('V') if !allow_void => {
            return Err(DescError::new(
                dims,
                1,
                "void is only valid as a return type",
            ))
        }
        Some(c @ ('V' | 'Z' | 'B' | 'S' | 'C' | 'I' | 'J' | 'F' | 'D')) => (TypeDesc::Prim(c), 1),
        Some('L') => {
            let Some(end) = rest.find(';') else {
                return Err(DescError::new(
                    dims,
                    rest.len(),
                    "class type is missing ';'",
                ));
            };
            let name = &rest[1..end];
            if name.is_empty() {
                return Err(DescError::new(dims, end + 1, "empty class name"));
            }
            for (i, part) in name.split('/').enumerate() {
                if part.is_empty() {
                    return Err(DescError::new(dims, end + 1, "empty segment in class name"));
                }
                if let Some(bad) = part.chars().find(|c| !valid_simple_name_char(*c)) {
                    let _ = i;
                    return Err(DescError::new(
                        dims,
                        end + 1,
                        format!("invalid character '{bad}' in class name"),
                    ));
                }
            }
            (TypeDesc::Class(name.to_string()), end + 1)
        }
        Some(c) => {
            return Err(DescError::new(
                dims,
                c.len_utf8(),
                format!("'{c}' does not start a type"),
            ))
        }
    };
    let ty = if dims > 0 {
        TypeDesc::Array(dims as u8, Box::new(base))
    } else {
        base
    };
    Ok((ty, dims + used))
}

/// Parse a complete type descriptor.
pub fn parse_type(s: &str, allow_void: bool) -> Result<TypeDesc, DescError> {
    let (t, used) = parse_type_prefix(s, allow_void)?;
    if used != s.len() {
        return Err(DescError::new(
            used,
            s.len() - used,
            "unexpected text after type",
        ));
    }
    Ok(t)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Proto {
    pub params: Vec<TypeDesc>,
    pub ret: TypeDesc,
}

impl Proto {
    /// Registers taken by the parameters (without `this`).
    pub fn param_slots(&self) -> u32 {
        self.params.iter().map(TypeDesc::slots).sum()
    }

    pub fn descriptor(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Proto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;
        for p in &self.params {
            write!(f, "{p}")?;
        }
        write!(f, "){}", self.ret)
    }
}

/// Parse `(params)ret`.
pub fn parse_proto(s: &str) -> Result<Proto, DescError> {
    if !s.starts_with('(') {
        return Err(DescError::new(0, 1, "expected '('"));
    }
    let Some(close) = s.find(')') else {
        return Err(DescError::new(0, s.len(), "missing ')'"));
    };
    let mut params = Vec::new();
    let mut pos = 1;
    while pos < close {
        let (t, used) = parse_type_prefix(&s[pos..close], false).map_err(|mut e| {
            e.offset += pos;
            e
        })?;
        params.push(t);
        pos += used;
    }
    let ret = parse_type(&s[close + 1..], true).map_err(|mut e| {
        e.offset += close + 1;
        e
    })?;
    Ok(Proto { params, ret })
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodRef {
    pub class: TypeDesc,
    pub name: String,
    pub proto: Proto,
}

impl MethodRef {
    pub fn descriptor(&self) -> String {
        format!("{}->{}{}", self.class, self.name, self.proto)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FieldRef {
    pub class: TypeDesc,
    pub name: String,
    pub ty: TypeDesc,
}

impl FieldRef {
    pub fn descriptor(&self) -> String {
        format!("{}->{}:{}", self.class, self.name, self.ty)
    }
}

fn split_owner(s: &str) -> Result<(TypeDesc, &str, usize), DescError> {
    let Some(arrow) = s.find("->") else {
        return Err(DescError::new(0, s.len(), "expected '->'"));
    };
    let class = parse_type(&s[..arrow], false)?;
    if !class.is_reference() {
        return Err(DescError::new(
            0,
            arrow,
            "member owner must be a class or array type",
        ));
    }
    Ok((class, &s[arrow + 2..], arrow + 2))
}

/// Parse `Lcls;->name(params)ret`.
pub fn parse_method_ref(s: &str) -> Result<MethodRef, DescError> {
    let (class, rest, base) = split_owner(s)?;
    let Some(paren) = rest.find('(') else {
        return Err(DescError::new(
            base,
            rest.len(),
            "expected '(' in method reference",
        ));
    };
    let name = &rest[..paren];
    if name.is_empty() {
        return Err(DescError::new(base, 1, "empty method name"));
    }
    let proto = parse_proto(&rest[paren..]).map_err(|mut e| {
        e.offset += base + paren;
        e
    })?;
    Ok(MethodRef {
        class,
        name: name.to_string(),
        proto,
    })
}

/// Parse `Lcls;->name:type`.
pub fn parse_field_ref(s: &str) -> Result<FieldRef, DescError> {
    let (class, rest, base) = split_owner(s)?;
    let Some(colon) = rest.find(':') else {
        return Err(DescError::new(
            base,
            rest.len(),
            "expected ':' in field reference",
        ));
    };
    let name = &rest[..colon];
    if name.is_empty() {
        return Err(DescError::new(base, 1, "empty field name"));
    }
    let ty = parse_type(&rest[colon + 1..], false).map_err(|mut e| {
        e.offset += base + colon + 1;
        e
    })?;
    Ok(FieldRef {
        class,
        name: name.to_string(),
        ty,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_and_classes() {
        for p in "ZBSCIJFD".chars() {
            assert_eq!(parse_type(&p.to_string(), false), Ok(TypeDesc::Prim(p)));
        }
        assert_eq!(parse_type("V", true), Ok(TypeDesc::Prim('V')));
        let t = parse_type("Lcom/foo/Bar$Inner$1;", false);
        assert_eq!(t, Ok(TypeDesc::Class("com/foo/Bar$Inner$1".into())));
        assert_eq!(
            t.map(|t| t.java(false)),
            Ok("com.foo.Bar.Inner.1".to_string())
        );
    }

    #[test]
    fn arrays() {
        let t = parse_type("[[Ljava/lang/String;", false);
        assert_eq!(t.as_ref().map(|t| t.java(true)), Ok("String[][]".into()));
        assert_eq!(t.map(|t| t.to_string()), Ok("[[Ljava/lang/String;".into()));
        assert!(parse_type(&format!("{}I", "[".repeat(255)), false).is_ok());
    }

    #[test]
    fn wide_params() {
        let p = parse_proto("(IJLjava/lang/Object;D[J)V");
        assert_eq!(p.map(|p| p.param_slots()), Ok(1 + 2 + 1 + 2 + 1));
    }

    #[test]
    fn s105_failures() {
        let bad: &[(&str, bool)] = &[
            ("", false),
            ("V", false),
            ("[V", true),
            ("Q", false),
            ("Ljava/lang/String", false),
            ("L;", false),
            ("Ljava//String;", false),
            ("Ljava/lang.String;", false),
            ("II", false),
        ];
        for (s, void) in bad {
            assert!(parse_type(s, *void).is_err(), "{s} should fail");
        }
        assert!(parse_type(&format!("{}I", "[".repeat(256)), false).is_err());
        assert!(parse_proto("(V)V").is_err());
        assert!(parse_proto("(I").is_err());
        assert!(parse_proto("()").is_err());
        assert!(parse_method_ref("Lfoo;->()V").is_err());
        assert!(parse_method_ref("I->a()V").is_err());
        assert!(parse_field_ref("Lfoo;->a:V").is_err());
    }

    #[test]
    fn member_refs() {
        let m = parse_method_ref(
            "Landroid/widget/Toast;->makeText(Landroid/content/Context;Ljava/lang/CharSequence;I)Landroid/widget/Toast;",
        );
        assert_eq!(m.as_ref().map(|m| m.name.as_str()), Ok("makeText"));
        assert_eq!(m.map(|m| m.proto.params.len()), Ok(3));
        let f = parse_field_ref("[I->length:I");
        assert!(f.is_ok());
        let e = parse_method_ref("Lfoo;->bar(Q)V").err();
        assert_eq!(e.map(|e| e.offset), Some(11));
    }
}

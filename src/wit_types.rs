//! W1.3: WIT Type System — Maps WIT types to Fajar Lang types.
//!
//! Provides bidirectional type mapping between WIT primitive/composite types
//! and Fajar Lang's type system. Also handles flags bitwise operations,
//! record field resolution, and variant pattern matching.

use super::wit_parser::{
    WitDocument, WitInterfaceDef, WitInterfaceItem, WitPrimitive, WitRecordField, WitTypeDef,
    WitTypeDefKind, WitTypeRef, WitUseDecl, WitUseName, WitVariantCase,
};
use std::collections::HashMap;
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// Fajar Type Representation
// ═══════════════════════════════════════════════════════════════════════

/// A Fajar Lang type that a WIT type maps to.
#[derive(Debug, Clone, PartialEq)]
pub enum FajarType {
    /// `u8`, `u16`, `u32`, `u64`
    UInt(u8),
    /// `i8`, `i16`, `i32`, `i64` (WIT: s8, s16, s32, s64)
    Int(u8),
    /// `f32`, `f64`
    Float(u8),
    /// `bool`
    Bool,
    /// `char`
    Char,
    /// `str` (Fajar's string type)
    Str,
    /// `Array<T>`
    Array(Box<FajarType>),
    /// `Option<T>`
    Option(Box<FajarType>),
    /// `Result<T, E>`
    Result {
        ok: Option<Box<FajarType>>,
        err: Option<Box<FajarType>>,
    },
    /// Tuple `(A, B, C)`
    Tuple(Vec<FajarType>),
    /// A Fajar struct by name
    Struct(String),
    /// A Fajar enum by name
    Enum(String),
    /// Bitflags type (stored as u32/u64)
    Flags { name: String, members: Vec<String> },
    /// Resource handle (opaque u32 index)
    ResourceHandle(String),
    /// `void` (no return)
    Void,
}

impl fmt::Display for FajarType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UInt(bits) => write!(f, "u{bits}"),
            Self::Int(bits) => write!(f, "i{bits}"),
            Self::Float(bits) => write!(f, "f{bits}"),
            Self::Bool => write!(f, "bool"),
            Self::Char => write!(f, "char"),
            Self::Str => write!(f, "str"),
            Self::Array(inner) => write!(f, "Array<{inner}>"),
            Self::Option(inner) => write!(f, "Option<{inner}>"),
            Self::Result { ok, err } => {
                write!(f, "Result<")?;
                match ok {
                    Some(t) => write!(f, "{t}")?,
                    None => write!(f, "()")?,
                }
                write!(f, ", ")?;
                match err {
                    Some(t) => write!(f, "{t}")?,
                    None => write!(f, "()")?,
                }
                write!(f, ">")
            }
            Self::Tuple(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
            Self::Struct(name) => write!(f, "{name}"),
            Self::Enum(name) => write!(f, "{name}"),
            Self::Flags { name, .. } => write!(f, "{name}"),
            Self::ResourceHandle(name) => write!(f, "Handle<{name}>"),
            Self::Void => write!(f, "void"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Type Mapper
// ═══════════════════════════════════════════════════════════════════════

/// Maps WIT types to Fajar Lang types, tracking resolved type definitions.
pub struct WitTypeMapper {
    /// Resolved named types: WIT name -> Fajar type.
    type_defs: HashMap<String, FajarType>,
    /// Resolved imports: alias -> original name.
    imports: HashMap<String, String>,
}

impl WitTypeMapper {
    /// Creates a new type mapper.
    pub fn new() -> Self {
        Self {
            type_defs: HashMap::new(),
            imports: HashMap::new(),
        }
    }

    /// Register all type definitions from a WIT document.
    pub fn register_document(&mut self, doc: &WitDocument) {
        // Register top-level use imports
        for u in &doc.top_use {
            self.register_use(u);
        }

        // Register types from all interfaces
        for iface in &doc.interfaces {
            self.register_interface(iface);
        }
    }

    /// Register types from an interface.
    pub fn register_interface(&mut self, iface: &WitInterfaceDef) {
        for item in &iface.items {
            match item {
                WitInterfaceItem::TypeDef(td) => self.register_type_def(td),
                WitInterfaceItem::Use(u) => self.register_use(u),
                WitInterfaceItem::Func(_) => {} // Functions don't define types
            }
        }
    }

    /// Register a `use` import.
    fn register_use(&mut self, u: &WitUseDecl) {
        for WitUseName { name, alias } in &u.names {
            let target = alias.as_deref().unwrap_or(name);
            self.imports.insert(target.to_string(), name.clone());
        }
    }

    /// Register a type definition.
    fn register_type_def(&mut self, td: &WitTypeDef) {
        let fajar_type = match &td.kind {
            WitTypeDefKind::Record(_fields) => {
                // Convert record name to PascalCase for Fajar struct
                let struct_name = wit_to_pascal_case(&td.name);
                // Pre-register so recursive types work
                self.type_defs
                    .insert(td.name.clone(), FajarType::Struct(struct_name.clone()));
                FajarType::Struct(struct_name)
            }
            WitTypeDefKind::Enum(_cases) => {
                let enum_name = wit_to_pascal_case(&td.name);
                FajarType::Enum(enum_name)
            }
            WitTypeDefKind::Variant(_cases) => {
                let enum_name = wit_to_pascal_case(&td.name);
                FajarType::Enum(enum_name)
            }
            WitTypeDefKind::Flags(members) => FajarType::Flags {
                name: wit_to_pascal_case(&td.name),
                members: members.clone(),
            },
            WitTypeDefKind::Resource(_) => FajarType::ResourceHandle(wit_to_pascal_case(&td.name)),
            WitTypeDefKind::Alias(target) => self.map_type_ref(target),
        };

        self.type_defs.insert(td.name.clone(), fajar_type);
    }

    /// Map a WIT type reference to a Fajar type.
    pub fn map_type_ref(&self, ty: &WitTypeRef) -> FajarType {
        match ty {
            WitTypeRef::Primitive(p) => map_primitive(p),
            WitTypeRef::List(inner) => FajarType::Array(Box::new(self.map_type_ref(inner))),
            WitTypeRef::Option(inner) => FajarType::Option(Box::new(self.map_type_ref(inner))),
            WitTypeRef::Result { ok, err } => FajarType::Result {
                ok: ok.as_ref().map(|t| Box::new(self.map_type_ref(t))),
                err: err.as_ref().map(|t| Box::new(self.map_type_ref(t))),
            },
            WitTypeRef::Tuple(items) => {
                FajarType::Tuple(items.iter().map(|t| self.map_type_ref(t)).collect())
            }
            WitTypeRef::Own(inner) => {
                // own<T> maps to T (ownership is the default in Fajar)
                self.map_type_ref(inner)
            }
            WitTypeRef::Borrow(inner) => {
                // borrow<T> maps to &T (reference in Fajar)
                self.map_type_ref(inner)
            }
            WitTypeRef::Named(name) => {
                // Check registered types first
                if let Some(ty) = self.type_defs.get(name.as_str()) {
                    return ty.clone();
                }
                // Check imports
                if let Some(original) = self.imports.get(name.as_str()) {
                    if let Some(ty) = self.type_defs.get(original.as_str()) {
                        return ty.clone();
                    }
                }
                // Unknown type — use as Fajar struct name
                FajarType::Struct(wit_to_pascal_case(name))
            }
        }
    }

    /// Map a record's fields to Fajar struct fields.
    pub fn map_record_fields(&self, fields: &[WitRecordField]) -> Vec<(String, FajarType)> {
        fields
            .iter()
            .map(|f| {
                let fajar_name = wit_to_snake_case(&f.name);
                let fajar_type = self.map_type_ref(&f.ty);
                (fajar_name, fajar_type)
            })
            .collect()
    }

    /// Map variant cases to Fajar enum variants.
    pub fn map_variant_cases(&self, cases: &[WitVariantCase]) -> Vec<(String, Option<FajarType>)> {
        cases
            .iter()
            .map(|c| {
                let fajar_name = wit_to_pascal_case(&c.name);
                let payload = c.ty.as_ref().map(|t| self.map_type_ref(t));
                (fajar_name, payload)
            })
            .collect()
    }

    /// Get a registered type by WIT name.
    pub fn get_type(&self, name: &str) -> Option<&FajarType> {
        self.type_defs.get(name)
    }

    /// Get all registered types.
    pub fn registered_types(&self) -> &HashMap<String, FajarType> {
        &self.type_defs
    }
}

impl Default for WitTypeMapper {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Flags Operations
// ═══════════════════════════════════════════════════════════════════════

/// Compile-time flag operations for WIT flags types.
#[derive(Debug, Clone)]
pub struct FlagSet {
    /// The flags type name.
    pub name: String,
    /// All available flags.
    pub members: Vec<String>,
    /// Current value as a bitmask.
    pub value: u64,
}

impl FlagSet {
    /// Creates a new empty flag set.
    pub fn new(name: &str, members: Vec<String>) -> Self {
        Self {
            name: name.to_string(),
            members,
            value: 0,
        }
    }

    /// Sets a flag by name.
    pub fn set(&mut self, flag: &str) -> bool {
        if let Some(idx) = self.members.iter().position(|m| m == flag) {
            self.value |= 1 << idx;
            true
        } else {
            false
        }
    }

    /// Clears a flag by name.
    pub fn clear(&mut self, flag: &str) -> bool {
        if let Some(idx) = self.members.iter().position(|m| m == flag) {
            self.value &= !(1 << idx);
            true
        } else {
            false
        }
    }

    /// Checks if a flag is set.
    pub fn contains(&self, flag: &str) -> bool {
        if let Some(idx) = self.members.iter().position(|m| m == flag) {
            (self.value & (1 << idx)) != 0
        } else {
            false
        }
    }

    /// Bitwise OR of two flag sets.
    pub fn union(&self, other: &FlagSet) -> FlagSet {
        FlagSet {
            name: self.name.clone(),
            members: self.members.clone(),
            value: self.value | other.value,
        }
    }

    /// Bitwise AND of two flag sets.
    pub fn intersection(&self, other: &FlagSet) -> FlagSet {
        FlagSet {
            name: self.name.clone(),
            members: self.members.clone(),
            value: self.value & other.value,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Naming Conventions
// ═══════════════════════════════════════════════════════════════════════

/// Convert WIT kebab-case to Fajar PascalCase: `input-stream` -> `InputStream`.
pub fn wit_to_pascal_case(name: &str) -> String {
    name.split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + chars.as_str()
                }
                None => String::new(),
            }
        })
        .collect()
}

/// Convert WIT kebab-case to Fajar snake_case: `input-stream` -> `input_stream`.
pub fn wit_to_snake_case(name: &str) -> String {
    name.replace('-', "_")
}

// ═══════════════════════════════════════════════════════════════════════
// Primitive Mapping
// ═══════════════════════════════════════════════════════════════════════

/// Map a WIT primitive to a Fajar type.
pub fn map_primitive(p: &WitPrimitive) -> FajarType {
    match p {
        WitPrimitive::U8 => FajarType::UInt(8),
        WitPrimitive::U16 => FajarType::UInt(16),
        WitPrimitive::U32 => FajarType::UInt(32),
        WitPrimitive::U64 => FajarType::UInt(64),
        WitPrimitive::S8 => FajarType::Int(8),
        WitPrimitive::S16 => FajarType::Int(16),
        WitPrimitive::S32 => FajarType::Int(32),
        WitPrimitive::S64 => FajarType::Int(64),
        WitPrimitive::F32 => FajarType::Float(32),
        WitPrimitive::F64 => FajarType::Float(64),
        WitPrimitive::Bool => FajarType::Bool,
        WitPrimitive::Char => FajarType::Char,
        WitPrimitive::String_ => FajarType::Str,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wit_parser::parse_wit;

    // ── W1.3: All 15 primitive types mapped ──

    #[test]
    fn w1_3_map_all_primitives() {
        assert_eq!(map_primitive(&WitPrimitive::U8), FajarType::UInt(8));
        assert_eq!(map_primitive(&WitPrimitive::U16), FajarType::UInt(16));
        assert_eq!(map_primitive(&WitPrimitive::U32), FajarType::UInt(32));
        assert_eq!(map_primitive(&WitPrimitive::U64), FajarType::UInt(64));
        assert_eq!(map_primitive(&WitPrimitive::S8), FajarType::Int(8));
        assert_eq!(map_primitive(&WitPrimitive::S16), FajarType::Int(16));
        assert_eq!(map_primitive(&WitPrimitive::S32), FajarType::Int(32));
        assert_eq!(map_primitive(&WitPrimitive::S64), FajarType::Int(64));
        assert_eq!(map_primitive(&WitPrimitive::F32), FajarType::Float(32));
        assert_eq!(map_primitive(&WitPrimitive::F64), FajarType::Float(64));
        assert_eq!(map_primitive(&WitPrimitive::Bool), FajarType::Bool);
        assert_eq!(map_primitive(&WitPrimitive::Char), FajarType::Char);
        assert_eq!(map_primitive(&WitPrimitive::String_), FajarType::Str);
    }

    #[test]
    fn w1_3_map_list_type() {
        let mapper = WitTypeMapper::new();
        let ty = mapper.map_type_ref(&WitTypeRef::List(Box::new(WitTypeRef::Primitive(
            WitPrimitive::U8,
        ))));
        assert_eq!(ty, FajarType::Array(Box::new(FajarType::UInt(8))));
        assert_eq!(ty.to_string(), "Array<u8>");
    }

    #[test]
    fn w1_3_map_option_string() {
        let mapper = WitTypeMapper::new();
        let ty = mapper.map_type_ref(&WitTypeRef::Option(Box::new(WitTypeRef::Primitive(
            WitPrimitive::String_,
        ))));
        assert_eq!(ty, FajarType::Option(Box::new(FajarType::Str)));
        assert_eq!(ty.to_string(), "Option<str>");
    }

    #[test]
    fn w1_3_map_result_type() {
        let mapper = WitTypeMapper::new();
        let ty = mapper.map_type_ref(&WitTypeRef::Result {
            ok: Some(Box::new(WitTypeRef::Primitive(WitPrimitive::U32))),
            err: Some(Box::new(WitTypeRef::Primitive(WitPrimitive::String_))),
        });
        assert_eq!(
            ty,
            FajarType::Result {
                ok: Some(Box::new(FajarType::UInt(32))),
                err: Some(Box::new(FajarType::Str)),
            }
        );
    }

    // ── W1.4: Record fields accessible ──

    #[test]
    fn w1_4_record_field_mapping() {
        let src = r#"
interface geo {
    record point {
        x: f64,
        y: f64,
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        let mut mapper = WitTypeMapper::new();
        mapper.register_document(&doc);

        let ty = mapper.get_type("point").unwrap();
        assert_eq!(ty, &FajarType::Struct("Point".into()));
    }

    #[test]
    fn w1_4_record_field_names_snake_case() {
        let fields = vec![
            WitRecordField {
                name: "file-name".into(),
                ty: WitTypeRef::Primitive(WitPrimitive::String_),
                doc: None,
            },
            WitRecordField {
                name: "byte-count".into(),
                ty: WitTypeRef::Primitive(WitPrimitive::U64),
                doc: None,
            },
        ];
        let mapper = WitTypeMapper::new();
        let mapped = mapper.map_record_fields(&fields);
        assert_eq!(mapped[0].0, "file_name");
        assert_eq!(mapped[0].1, FajarType::Str);
        assert_eq!(mapped[1].0, "byte_count");
        assert_eq!(mapped[1].1, FajarType::UInt(64));
    }

    // ── W1.5: Variant matching ──

    #[test]
    fn w1_5_variant_case_mapping() {
        let cases = vec![
            WitVariantCase {
                name: "timeout".into(),
                ty: None,
                doc: None,
            },
            WitVariantCase {
                name: "refused".into(),
                ty: Some(WitTypeRef::Primitive(WitPrimitive::String_)),
                doc: None,
            },
            WitVariantCase {
                name: "other-error".into(),
                ty: Some(WitTypeRef::Primitive(WitPrimitive::U32)),
                doc: None,
            },
        ];
        let mapper = WitTypeMapper::new();
        let mapped = mapper.map_variant_cases(&cases);
        assert_eq!(mapped[0].0, "Timeout");
        assert!(mapped[0].1.is_none());
        assert_eq!(mapped[1].0, "Refused");
        assert_eq!(mapped[1].1, Some(FajarType::Str));
        assert_eq!(mapped[2].0, "OtherError");
        assert_eq!(mapped[2].1, Some(FajarType::UInt(32)));
    }

    // ── W1.6: Flags operations ──

    #[test]
    fn w1_6_flags_set_contains() {
        let mut flags = FlagSet::new(
            "Permissions",
            vec!["read".into(), "write".into(), "exec".into()],
        );
        assert!(!flags.contains("read"));
        assert!(flags.set("read"));
        assert!(flags.contains("read"));
        assert!(!flags.contains("write"));
        assert!(flags.set("write"));
        assert!(flags.contains("write"));
    }

    #[test]
    fn w1_6_flags_union_and_intersection() {
        let mut a = FlagSet::new("Perm", vec!["read".into(), "write".into(), "exec".into()]);
        let mut b = FlagSet::new("Perm", vec!["read".into(), "write".into(), "exec".into()]);
        a.set("read");
        a.set("exec");
        b.set("read");
        b.set("write");

        let union = a.union(&b);
        assert!(union.contains("read"));
        assert!(union.contains("write"));
        assert!(union.contains("exec"));

        let inter = a.intersection(&b);
        assert!(inter.contains("read"));
        assert!(!inter.contains("write"));
        assert!(!inter.contains("exec"));
    }

    #[test]
    fn w1_6_flags_clear() {
        let mut flags = FlagSet::new("Perm", vec!["read".into(), "write".into()]);
        flags.set("read");
        flags.set("write");
        assert!(flags.contains("read"));
        flags.clear("read");
        assert!(!flags.contains("read"));
        assert!(flags.contains("write"));
    }

    #[test]
    fn w1_6_flags_type_mapping() {
        let src = r#"
interface fs {
    flags permissions { read, write, exec }
}
"#;
        let doc = parse_wit(src).unwrap();
        let mut mapper = WitTypeMapper::new();
        mapper.register_document(&doc);
        let ty = mapper.get_type("permissions").unwrap();
        if let FajarType::Flags { name, members } = ty {
            assert_eq!(name, "Permissions");
            assert_eq!(members, &["read", "write", "exec"]);
        } else {
            panic!("expected Flags type");
        }
    }

    // ── W1.7: Resource handle mapping ──

    #[test]
    fn w1_7_resource_maps_to_handle() {
        let src = r#"
interface streams {
    resource input-stream;
}
"#;
        let doc = parse_wit(src).unwrap();
        let mut mapper = WitTypeMapper::new();
        mapper.register_document(&doc);
        let ty = mapper.get_type("input-stream").unwrap();
        assert_eq!(ty, &FajarType::ResourceHandle("InputStream".into()));
        assert_eq!(ty.to_string(), "Handle<InputStream>");
    }

    // ── W1.8: Tuple/Option/Result mapping ──

    #[test]
    fn w1_8_option_maps_to_fajar_option() {
        let mapper = WitTypeMapper::new();
        let ty = mapper.map_type_ref(&WitTypeRef::Option(Box::new(WitTypeRef::Primitive(
            WitPrimitive::String_,
        ))));
        assert_eq!(ty.to_string(), "Option<str>");
    }

    #[test]
    fn w1_8_result_maps_to_fajar_result() {
        let mapper = WitTypeMapper::new();
        let ty = mapper.map_type_ref(&WitTypeRef::Result {
            ok: Some(Box::new(WitTypeRef::List(Box::new(WitTypeRef::Primitive(
                WitPrimitive::U8,
            ))))),
            err: Some(Box::new(WitTypeRef::Primitive(WitPrimitive::String_))),
        });
        assert_eq!(ty.to_string(), "Result<Array<u8>, str>");
    }

    #[test]
    fn w1_8_tuple_maps_to_fajar_tuple() {
        let mapper = WitTypeMapper::new();
        let ty = mapper.map_type_ref(&WitTypeRef::Tuple(vec![
            WitTypeRef::Primitive(WitPrimitive::F64),
            WitTypeRef::Primitive(WitPrimitive::F64),
            WitTypeRef::Primitive(WitPrimitive::F64),
        ]));
        assert_eq!(ty.to_string(), "(f64, f64, f64)");
    }

    // ── W1.9: Use imports resolve ──

    #[test]
    fn w1_9_use_import_resolves_type() {
        let src = r#"
interface types {
    record point {
        x: f64,
        y: f64,
    }
}

interface consumer {
    use types.{point};
    get-point: func() -> point;
}
"#;
        let doc = parse_wit(src).unwrap();
        let mut mapper = WitTypeMapper::new();
        mapper.register_document(&doc);

        // `point` should resolve to the registered struct
        let ty = mapper.get_type("point").unwrap();
        assert_eq!(ty, &FajarType::Struct("Point".into()));
    }

    #[test]
    fn w1_9_use_import_with_alias() {
        let src = r#"
interface types {
    record point { x: f64, y: f64 }
}

interface consumer {
    use types.{point as pt};
}
"#;
        let doc = parse_wit(src).unwrap();
        let mut mapper = WitTypeMapper::new();
        mapper.register_document(&doc);

        // The alias `pt` should also resolve
        assert!(mapper.imports.contains_key("pt"));
        assert_eq!(mapper.imports["pt"], "point");
    }

    // ── Naming convention tests ──

    #[test]
    fn w1_naming_pascal_case() {
        assert_eq!(wit_to_pascal_case("input-stream"), "InputStream");
        assert_eq!(wit_to_pascal_case("tcp-socket"), "TcpSocket");
        assert_eq!(wit_to_pascal_case("path-flags"), "PathFlags");
        assert_eq!(wit_to_pascal_case("simple"), "Simple");
    }

    #[test]
    fn w1_naming_snake_case() {
        assert_eq!(wit_to_snake_case("file-name"), "file_name");
        assert_eq!(wit_to_snake_case("byte-count"), "byte_count");
        assert_eq!(wit_to_snake_case("simple"), "simple");
    }

    // ── W1.10: Full integration — map entire WASI HTTP types ──

    #[test]
    fn w1_10_full_wasi_http_type_mapping() {
        let src = r#"
package wasi:http@0.2.0;

interface types {
    record request {
        method: string,
        url: string,
        headers: list<tuple<string, string>>,
        body: option<list<u8>>,
    }

    record response {
        status: u16,
        headers: list<tuple<string, string>>,
        body: list<u8>,
    }

    variant error {
        network-error(string),
        timeout,
        dns-error(string),
    }

    flags method-filter {
        get,
        post,
        put,
        delete,
    }

    resource connection;
}
"#;
        let doc = parse_wit(src).unwrap();
        let mut mapper = WitTypeMapper::new();
        mapper.register_document(&doc);

        // Check all types were registered
        assert_eq!(
            mapper.get_type("request").unwrap(),
            &FajarType::Struct("Request".into())
        );
        assert_eq!(
            mapper.get_type("response").unwrap(),
            &FajarType::Struct("Response".into())
        );
        assert_eq!(
            mapper.get_type("error").unwrap(),
            &FajarType::Enum("Error".into())
        );
        if let Some(FajarType::Flags { name, members }) = mapper.get_type("method-filter") {
            assert_eq!(name, "MethodFilter");
            assert_eq!(members.len(), 4);
        } else {
            panic!("expected flags");
        }
        assert_eq!(
            mapper.get_type("connection").unwrap(),
            &FajarType::ResourceHandle("Connection".into())
        );

        // Total: 5 types registered
        assert_eq!(mapper.registered_types().len(), 5);
    }
}

//! W1.2–W1.9: WIT Parser — Recursive-descent parser for `.wit` files.
//!
//! Produces a `WitDocument` containing interfaces, worlds, type definitions,
//! and `use` imports. Supports all WIT constructs:
//! - Records (W1.4), Variants (W1.5), Flags (W1.6), Resources (W1.7)
//! - Tuple/Option/Result (W1.8), Use imports (W1.9)

#![allow(missing_docs)] // P6.E4: data-heavy enum/struct module; field+variant names self-document

use super::wit_lexer::{WitToken, WitTokenKind, tokenize_wit};
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// AST Types
// ═══════════════════════════════════════════════════════════════════════

/// A complete WIT document parsed from a `.wit` file.
#[derive(Debug, Clone)]
pub struct WitDocument {
    /// Package declaration (e.g., `wasi:cli@0.2.0`).
    pub package: Option<WitPackage>,
    /// Top-level interfaces.
    pub interfaces: Vec<WitInterfaceDef>,
    /// Top-level worlds.
    pub worlds: Vec<WitWorldDef>,
    /// Top-level `use` imports outside interfaces/worlds.
    pub top_use: Vec<WitUseDecl>,
}

/// Package declaration: `package namespace:name@version;`
#[derive(Debug, Clone, PartialEq)]
pub struct WitPackage {
    /// Namespace (e.g., `wasi`).
    pub namespace: String,
    /// Package name (e.g., `cli`).
    pub name: String,
    /// Optional semver version.
    pub version: Option<String>,
}

impl fmt::Display for WitPackage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.name)?;
        if let Some(v) = &self.version {
            write!(f, "@{v}")?;
        }
        Ok(())
    }
}

/// An interface definition: `interface name { ... }`
#[derive(Debug, Clone)]
pub struct WitInterfaceDef {
    /// Interface name.
    pub name: String,
    /// Doc comment, if any.
    pub doc: Option<String>,
    /// Items inside the interface.
    pub items: Vec<WitInterfaceItem>,
}

/// Items that can appear inside an interface block.
#[derive(Debug, Clone)]
pub enum WitInterfaceItem {
    /// A function definition.
    Func(WitFuncDef),
    /// A type definition (record, enum, variant, flags, resource, type alias).
    TypeDef(WitTypeDef),
    /// A `use` import.
    Use(WitUseDecl),
}

/// A world definition: `world name { ... }`
#[derive(Debug, Clone)]
pub struct WitWorldDef {
    /// World name.
    pub name: String,
    /// Doc comment, if any.
    pub doc: Option<String>,
    /// Items inside the world.
    pub items: Vec<WitWorldItem>,
}

/// Items that can appear inside a world block.
#[derive(Debug, Clone)]
pub enum WitWorldItem {
    /// `import iface-path;` or `import name: func(...) -> ...;`
    Import(WitWorldImport),
    /// `export iface-path;` or `export name: func(...) -> ...;`
    Export(WitWorldExport),
    /// `include other-world;`
    Include(String),
    /// A `use` import.
    Use(WitUseDecl),
    /// Inline type definition.
    TypeDef(WitTypeDef),
}

/// World import: either an interface path or an inline function.
#[derive(Debug, Clone)]
pub enum WitWorldImport {
    /// Import an interface by path (e.g., `wasi:io/streams@0.2.0`).
    InterfacePath(WitExternPath),
    /// Import a named function inline.
    Func { name: String, func: WitFuncDef },
}

/// World export: either an interface path or an inline function.
#[derive(Debug, Clone)]
pub enum WitWorldExport {
    /// Export an interface by path.
    InterfacePath(WitExternPath),
    /// Export a named function inline.
    Func { name: String, func: WitFuncDef },
}

/// External interface path: `namespace:pkg/iface@version`
#[derive(Debug, Clone, PartialEq)]
pub struct WitExternPath {
    /// Full path text (e.g., `wasi:io/streams@0.2.0`).
    pub path: String,
}

impl fmt::Display for WitExternPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)
    }
}

/// A function definition: `name: func(params) -> result`
#[derive(Debug, Clone)]
pub struct WitFuncDef {
    /// Function name.
    pub name: String,
    /// Doc comment, if any.
    pub doc: Option<String>,
    /// Parameters.
    pub params: Vec<WitParam>,
    /// Return type (if any).
    pub result: Option<WitTypeRef>,
    /// Whether this is a static method.
    pub is_static: bool,
    /// Whether this is a constructor.
    pub is_constructor: bool,
}

/// A function parameter: `name: type`
#[derive(Debug, Clone, PartialEq)]
pub struct WitParam {
    /// Parameter name.
    pub name: String,
    /// Parameter type.
    pub ty: WitTypeRef,
}

/// A type definition.
#[derive(Debug, Clone)]
pub struct WitTypeDef {
    /// Type name.
    pub name: String,
    /// Doc comment, if any.
    pub doc: Option<String>,
    /// The type body.
    pub kind: WitTypeDefKind,
}

/// Kinds of type definitions.
#[derive(Debug, Clone)]
pub enum WitTypeDefKind {
    /// `record name { field: type, ... }`
    Record(Vec<WitRecordField>),
    /// `enum name { case1, case2, ... }`
    Enum(Vec<WitEnumCase>),
    /// `variant name { case1(type), case2, ... }`
    Variant(Vec<WitVariantCase>),
    /// `flags name { flag1, flag2, ... }`
    Flags(Vec<String>),
    /// `resource name { ... }`
    Resource(WitResourceDef),
    /// `type alias = other-type;`
    Alias(WitTypeRef),
}

/// A field in a record: `name: type`
#[derive(Debug, Clone, PartialEq)]
pub struct WitRecordField {
    pub name: String,
    pub ty: WitTypeRef,
    pub doc: Option<String>,
}

/// A case in an enum (no payload).
#[derive(Debug, Clone, PartialEq)]
pub struct WitEnumCase {
    pub name: String,
    pub doc: Option<String>,
}

/// A case in a variant (optional payload).
#[derive(Debug, Clone, PartialEq)]
pub struct WitVariantCase {
    pub name: String,
    pub ty: Option<WitTypeRef>,
    pub doc: Option<String>,
}

/// A resource definition with optional methods.
#[derive(Debug, Clone)]
pub struct WitResourceDef {
    /// Constructor, if any.
    pub constructor: Option<WitFuncDef>,
    /// Methods: `[method]self.name: func(...)`.
    pub methods: Vec<WitFuncDef>,
    /// Static functions: `[static]name: func(...)`.
    pub statics: Vec<WitFuncDef>,
}

/// A `use` declaration: `use iface-path.{name1, name2 as alias}`
#[derive(Debug, Clone, PartialEq)]
pub struct WitUseDecl {
    /// The interface path being imported from.
    pub from: WitExternPath,
    /// Names being imported, with optional aliases.
    pub names: Vec<WitUseName>,
}

/// A single name in a `use` declaration, with optional alias.
#[derive(Debug, Clone, PartialEq)]
pub struct WitUseName {
    /// Original name.
    pub name: String,
    /// Optional alias (`as new-name`).
    pub alias: Option<String>,
}

/// A reference to a type (could be primitive, generic, named, own, borrow).
#[derive(Debug, Clone, PartialEq)]
pub enum WitTypeRef {
    /// Primitive: `u8`, `u16`, `u32`, `u64`, `s8`, ... `f32`, `f64`, `bool`, `char`, `string`.
    Primitive(WitPrimitive),
    /// `list<T>`
    List(Box<WitTypeRef>),
    /// `option<T>`
    Option(Box<WitTypeRef>),
    /// `result<T, E>` or `result<_, E>` or `result<T>` or `result`
    Result {
        ok: Option<Box<WitTypeRef>>,
        err: Option<Box<WitTypeRef>>,
    },
    /// `tuple<A, B, C>`
    Tuple(Vec<WitTypeRef>),
    /// `own<T>` — ownership handle
    Own(Box<WitTypeRef>),
    /// `borrow<T>` — borrowed handle
    Borrow(Box<WitTypeRef>),
    /// A named type reference (user-defined or imported).
    Named(String),
}

impl fmt::Display for WitTypeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Primitive(p) => write!(f, "{p}"),
            Self::List(inner) => write!(f, "list<{inner}>"),
            Self::Option(inner) => write!(f, "option<{inner}>"),
            Self::Result { ok, err } => {
                write!(f, "result")?;
                match (ok, err) {
                    (Some(o), Some(e)) => write!(f, "<{o}, {e}>"),
                    (Some(o), None) => write!(f, "<{o}>"),
                    (None, Some(e)) => write!(f, "<_, {e}>"),
                    (None, None) => Ok(()),
                }
            }
            Self::Tuple(items) => {
                write!(f, "tuple<")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ">")
            }
            Self::Own(inner) => write!(f, "own<{inner}>"),
            Self::Borrow(inner) => write!(f, "borrow<{inner}>"),
            Self::Named(name) => write!(f, "{name}"),
        }
    }
}

/// WIT primitive types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WitPrimitive {
    U8,
    U16,
    U32,
    U64,
    S8,
    S16,
    S32,
    S64,
    F32,
    F64,
    Bool,
    Char,
    String_,
}

impl fmt::Display for WitPrimitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::U8 => write!(f, "u8"),
            Self::U16 => write!(f, "u16"),
            Self::U32 => write!(f, "u32"),
            Self::U64 => write!(f, "u64"),
            Self::S8 => write!(f, "s8"),
            Self::S16 => write!(f, "s16"),
            Self::S32 => write!(f, "s32"),
            Self::S64 => write!(f, "s64"),
            Self::F32 => write!(f, "f32"),
            Self::F64 => write!(f, "f64"),
            Self::Bool => write!(f, "bool"),
            Self::Char => write!(f, "char"),
            Self::String_ => write!(f, "string"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Parse Error
// ═══════════════════════════════════════════════════════════════════════

/// Error during WIT parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct WitParseError {
    /// Error message.
    pub message: String,
    /// Byte offset of the token that caused the error.
    pub offset: usize,
}

impl fmt::Display for WitParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "WIT parse error at offset {}: {}",
            self.offset, self.message
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Parser
// ═══════════════════════════════════════════════════════════════════════

/// Recursive-descent WIT parser.
pub struct WitParser {
    tokens: Vec<WitToken>,
    pos: usize,
}

impl WitParser {
    /// Creates a parser from a token stream.
    pub fn new(tokens: Vec<WitToken>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &WitTokenKind {
        self.tokens
            .get(self.pos)
            .map(|t| &t.kind)
            .unwrap_or(&WitTokenKind::Eof)
    }

    fn offset(&self) -> usize {
        self.tokens.get(self.pos).map(|t| t.offset).unwrap_or(0)
    }

    fn advance(&mut self) -> &WitToken {
        let tok = &self.tokens[self.pos];
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn expect(&mut self, expected: &WitTokenKind) -> Result<&WitToken, WitParseError> {
        if self.peek() == expected {
            Ok(self.advance())
        } else {
            Err(WitParseError {
                message: format!("expected `{expected}`, found `{}`", self.peek()),
                offset: self.offset(),
            })
        }
    }

    fn expect_ident(&mut self) -> Result<String, WitParseError> {
        match self.peek().clone() {
            WitTokenKind::Ident(s) => {
                let name = s.clone();
                self.advance();
                Ok(name)
            }
            other => Err(WitParseError {
                message: format!("expected identifier, found `{other}`"),
                offset: self.offset(),
            }),
        }
    }

    fn eat(&mut self, kind: &WitTokenKind) -> bool {
        if self.peek() == kind {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Consume a doc comment if present.
    fn take_doc(&mut self) -> Option<String> {
        if let WitTokenKind::DocComment(s) = self.peek().clone() {
            let doc = s.clone();
            self.advance();
            // Accumulate multiple consecutive doc comments
            let mut full = doc;
            while let WitTokenKind::DocComment(s) = self.peek().clone() {
                full.push('\n');
                full.push_str(&s);
                self.advance();
            }
            Some(full)
        } else {
            None
        }
    }

    // ── Top-level parse ──

    /// Parses a complete WIT document.
    pub fn parse_document(&mut self) -> Result<WitDocument, WitParseError> {
        let mut doc = WitDocument {
            package: None,
            interfaces: Vec::new(),
            worlds: Vec::new(),
            top_use: Vec::new(),
        };

        loop {
            let _doc_comment = self.take_doc();
            match self.peek().clone() {
                WitTokenKind::Eof => break,
                WitTokenKind::Package => {
                    doc.package = Some(self.parse_package()?);
                }
                WitTokenKind::Interface => {
                    doc.interfaces.push(self.parse_interface(_doc_comment)?);
                }
                WitTokenKind::World => {
                    doc.worlds.push(self.parse_world(_doc_comment)?);
                }
                WitTokenKind::Use => {
                    doc.top_use.push(self.parse_use()?);
                }
                other => {
                    return Err(WitParseError {
                        message: format!("unexpected top-level token: `{other}`"),
                        offset: self.offset(),
                    });
                }
            }
        }
        Ok(doc)
    }

    // ── Package ──

    fn parse_package(&mut self) -> Result<WitPackage, WitParseError> {
        self.expect(&WitTokenKind::Package)?;
        let namespace = self.expect_ident()?;
        self.expect(&WitTokenKind::Colon)?;
        let name = self.expect_ident()?;
        let version = if self.eat(&WitTokenKind::At) {
            match self.peek().clone() {
                WitTokenKind::SemVer(v) => {
                    let ver = v.clone();
                    self.advance();
                    Some(ver)
                }
                other => {
                    return Err(WitParseError {
                        message: format!("expected version after `@`, found `{other}`"),
                        offset: self.offset(),
                    });
                }
            }
        } else {
            None
        };
        self.expect(&WitTokenKind::Semicolon)?;
        Ok(WitPackage {
            namespace,
            name,
            version,
        })
    }

    // ── Interface ──

    fn parse_interface(&mut self, doc: Option<String>) -> Result<WitInterfaceDef, WitParseError> {
        self.expect(&WitTokenKind::Interface)?;
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::LBrace)?;

        let mut items = Vec::new();
        loop {
            let item_doc = self.take_doc();
            match self.peek().clone() {
                WitTokenKind::RBrace => {
                    self.advance();
                    break;
                }
                WitTokenKind::Record => {
                    items.push(WitInterfaceItem::TypeDef(self.parse_record(item_doc)?));
                }
                WitTokenKind::Enum => {
                    items.push(WitInterfaceItem::TypeDef(self.parse_enum(item_doc)?));
                }
                WitTokenKind::Variant => {
                    items.push(WitInterfaceItem::TypeDef(self.parse_variant(item_doc)?));
                }
                WitTokenKind::Flags => {
                    items.push(WitInterfaceItem::TypeDef(self.parse_flags(item_doc)?));
                }
                WitTokenKind::Resource => {
                    items.push(WitInterfaceItem::TypeDef(self.parse_resource(item_doc)?));
                }
                WitTokenKind::Type => {
                    items.push(WitInterfaceItem::TypeDef(self.parse_type_alias(item_doc)?));
                }
                WitTokenKind::Use => {
                    items.push(WitInterfaceItem::Use(self.parse_use()?));
                }
                WitTokenKind::Ident(_) => {
                    items.push(WitInterfaceItem::Func(self.parse_func_item(item_doc)?));
                }
                WitTokenKind::Constructor => {
                    items.push(WitInterfaceItem::Func(
                        self.parse_constructor_func(item_doc)?,
                    ));
                }
                WitTokenKind::Eof => {
                    return Err(WitParseError {
                        message: "unexpected end of file in interface".into(),
                        offset: self.offset(),
                    });
                }
                other => {
                    return Err(WitParseError {
                        message: format!("unexpected token in interface: `{other}`"),
                        offset: self.offset(),
                    });
                }
            }
        }

        Ok(WitInterfaceDef { name, doc, items })
    }

    // ── World ──

    fn parse_world(&mut self, doc: Option<String>) -> Result<WitWorldDef, WitParseError> {
        self.expect(&WitTokenKind::World)?;
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::LBrace)?;

        let mut items = Vec::new();
        loop {
            let _item_doc = self.take_doc();
            match self.peek().clone() {
                WitTokenKind::RBrace => {
                    self.advance();
                    break;
                }
                WitTokenKind::Import => {
                    items.push(WitWorldItem::Import(self.parse_world_import()?));
                }
                WitTokenKind::Export => {
                    items.push(WitWorldItem::Export(self.parse_world_export()?));
                }
                WitTokenKind::Include => {
                    self.advance();
                    let incl_name = self.expect_ident()?;
                    self.expect(&WitTokenKind::Semicolon)?;
                    items.push(WitWorldItem::Include(incl_name));
                }
                WitTokenKind::Use => {
                    items.push(WitWorldItem::Use(self.parse_use()?));
                }
                WitTokenKind::Record => {
                    items.push(WitWorldItem::TypeDef(self.parse_record(_item_doc)?));
                }
                WitTokenKind::Enum => {
                    items.push(WitWorldItem::TypeDef(self.parse_enum(_item_doc)?));
                }
                WitTokenKind::Variant => {
                    items.push(WitWorldItem::TypeDef(self.parse_variant(_item_doc)?));
                }
                WitTokenKind::Flags => {
                    items.push(WitWorldItem::TypeDef(self.parse_flags(_item_doc)?));
                }
                WitTokenKind::Type => {
                    items.push(WitWorldItem::TypeDef(self.parse_type_alias(_item_doc)?));
                }
                WitTokenKind::Eof => {
                    return Err(WitParseError {
                        message: "unexpected end of file in world".into(),
                        offset: self.offset(),
                    });
                }
                other => {
                    return Err(WitParseError {
                        message: format!("unexpected token in world: `{other}`"),
                        offset: self.offset(),
                    });
                }
            }
        }

        Ok(WitWorldDef { name, doc, items })
    }

    fn parse_world_import(&mut self) -> Result<WitWorldImport, WitParseError> {
        self.expect(&WitTokenKind::Import)?;

        // Could be: `import iface-path;` or `import name: func(...);`
        let name = self.expect_ident()?;

        if self.eat(&WitTokenKind::Colon) {
            // Check for `func(...)` inline
            if self.peek() == &WitTokenKind::Func {
                let func = self.parse_func_body(name.clone(), None)?;
                self.expect(&WitTokenKind::Semicolon)?;
                return Ok(WitWorldImport::Func { name, func });
            }
            // Otherwise it's a path like `wasi:io/streams@0.2.0`
            let path = self.parse_extern_path_rest(&name)?;
            self.expect(&WitTokenKind::Semicolon)?;
            Ok(WitWorldImport::InterfacePath(path))
        } else if self.peek() == &WitTokenKind::Semicolon {
            // Simple name reference
            self.advance();
            Ok(WitWorldImport::InterfacePath(WitExternPath { path: name }))
        } else {
            Err(WitParseError {
                message: "expected `:` or `;` after import name".into(),
                offset: self.offset(),
            })
        }
    }

    fn parse_world_export(&mut self) -> Result<WitWorldExport, WitParseError> {
        self.expect(&WitTokenKind::Export)?;

        let name = self.expect_ident()?;

        if self.eat(&WitTokenKind::Colon) {
            if self.peek() == &WitTokenKind::Func {
                let func = self.parse_func_body(name.clone(), None)?;
                self.expect(&WitTokenKind::Semicolon)?;
                return Ok(WitWorldExport::Func { name, func });
            }
            let path = self.parse_extern_path_rest(&name)?;
            self.expect(&WitTokenKind::Semicolon)?;
            Ok(WitWorldExport::InterfacePath(path))
        } else if self.peek() == &WitTokenKind::Semicolon {
            self.advance();
            Ok(WitWorldExport::InterfacePath(WitExternPath { path: name }))
        } else {
            Err(WitParseError {
                message: "expected `:` or `;` after export name".into(),
                offset: self.offset(),
            })
        }
    }

    /// Parse the rest of an extern path after we've consumed `name:`.
    /// e.g., after `wasi:` we expect `io/streams@0.2.0`.
    fn parse_extern_path_rest(&mut self, namespace: &str) -> Result<WitExternPath, WitParseError> {
        let mut path = format!("{namespace}:");
        // Expect package name
        let pkg = self.expect_ident()?;
        path.push_str(&pkg);
        // Optional `/interface`
        if self.eat(&WitTokenKind::Slash) {
            path.push('/');
            let iface = self.expect_ident()?;
            path.push_str(&iface);
        }
        // Optional `@version`
        if self.eat(&WitTokenKind::At) {
            if let WitTokenKind::SemVer(v) = self.peek().clone() {
                path.push('@');
                path.push_str(&v);
                self.advance();
            }
        }
        Ok(WitExternPath { path })
    }

    // ── Use ──

    fn parse_use(&mut self) -> Result<WitUseDecl, WitParseError> {
        self.expect(&WitTokenKind::Use)?;

        // Parse the path: `namespace:pkg/iface.{names}` or `iface.{names}`
        let first = self.expect_ident()?;
        let from = if self.eat(&WitTokenKind::Colon) {
            self.parse_extern_path_rest(&first)?
        } else {
            WitExternPath { path: first }
        };

        self.expect(&WitTokenKind::Dot)?;
        self.expect(&WitTokenKind::LBrace)?;

        let mut names = Vec::new();
        loop {
            if self.peek() == &WitTokenKind::RBrace {
                self.advance();
                break;
            }
            let name = self.expect_ident()?;
            let alias = if self.peek() == &WitTokenKind::Ident("as".into()) {
                self.advance();
                Some(self.expect_ident()?)
            } else {
                None
            };
            names.push(WitUseName { name, alias });

            if !self.eat(&WitTokenKind::Comma) {
                self.expect(&WitTokenKind::RBrace)?;
                break;
            }
        }

        self.expect(&WitTokenKind::Semicolon)?;
        Ok(WitUseDecl { from, names })
    }

    // ── Type definitions ──

    /// W1.4: Record — `record name { field: type, ... }`
    fn parse_record(&mut self, doc: Option<String>) -> Result<WitTypeDef, WitParseError> {
        self.expect(&WitTokenKind::Record)?;
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::LBrace)?;

        let mut fields = Vec::new();
        loop {
            let field_doc = self.take_doc();
            if self.peek() == &WitTokenKind::RBrace {
                self.advance();
                break;
            }
            let field_name = self.expect_ident()?;
            self.expect(&WitTokenKind::Colon)?;
            let ty = self.parse_type_ref()?;
            fields.push(WitRecordField {
                name: field_name,
                ty,
                doc: field_doc,
            });

            if !self.eat(&WitTokenKind::Comma) {
                self.expect(&WitTokenKind::RBrace)?;
                break;
            }
        }

        Ok(WitTypeDef {
            name,
            doc,
            kind: WitTypeDefKind::Record(fields),
        })
    }

    /// W1.5 (enum part): `enum name { case1, case2, ... }`
    fn parse_enum(&mut self, doc: Option<String>) -> Result<WitTypeDef, WitParseError> {
        self.expect(&WitTokenKind::Enum)?;
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::LBrace)?;

        let mut cases = Vec::new();
        loop {
            let case_doc = self.take_doc();
            if self.peek() == &WitTokenKind::RBrace {
                self.advance();
                break;
            }
            let case_name = self.expect_ident()?;
            cases.push(WitEnumCase {
                name: case_name,
                doc: case_doc,
            });
            if !self.eat(&WitTokenKind::Comma) {
                self.expect(&WitTokenKind::RBrace)?;
                break;
            }
        }

        Ok(WitTypeDef {
            name,
            doc,
            kind: WitTypeDefKind::Enum(cases),
        })
    }

    /// W1.5: Variant — `variant name { case1(type), case2, ... }`
    fn parse_variant(&mut self, doc: Option<String>) -> Result<WitTypeDef, WitParseError> {
        self.expect(&WitTokenKind::Variant)?;
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::LBrace)?;

        let mut cases = Vec::new();
        loop {
            let case_doc = self.take_doc();
            if self.peek() == &WitTokenKind::RBrace {
                self.advance();
                break;
            }
            let case_name = self.expect_ident()?;
            let payload = if self.eat(&WitTokenKind::LParen) {
                let ty = self.parse_type_ref()?;
                self.expect(&WitTokenKind::RParen)?;
                Some(ty)
            } else {
                None
            };
            cases.push(WitVariantCase {
                name: case_name,
                ty: payload,
                doc: case_doc,
            });
            if !self.eat(&WitTokenKind::Comma) {
                self.expect(&WitTokenKind::RBrace)?;
                break;
            }
        }

        Ok(WitTypeDef {
            name,
            doc,
            kind: WitTypeDefKind::Variant(cases),
        })
    }

    /// W1.6: Flags — `flags name { flag1, flag2, ... }`
    fn parse_flags(&mut self, doc: Option<String>) -> Result<WitTypeDef, WitParseError> {
        self.expect(&WitTokenKind::Flags)?;
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::LBrace)?;

        let mut flags = Vec::new();
        loop {
            let _flag_doc = self.take_doc();
            if self.peek() == &WitTokenKind::RBrace {
                self.advance();
                break;
            }
            let flag_name = self.expect_ident()?;
            flags.push(flag_name);
            if !self.eat(&WitTokenKind::Comma) {
                self.expect(&WitTokenKind::RBrace)?;
                break;
            }
        }

        Ok(WitTypeDef {
            name,
            doc,
            kind: WitTypeDefKind::Flags(flags),
        })
    }

    /// W1.7: Resource — `resource name { constructor(...); [method]self.name: func(...); }`
    fn parse_resource(&mut self, doc: Option<String>) -> Result<WitTypeDef, WitParseError> {
        self.expect(&WitTokenKind::Resource)?;
        let name = self.expect_ident()?;

        // Resource can be just `resource name;` (opaque) or `resource name { ... }`
        if self.eat(&WitTokenKind::Semicolon) {
            return Ok(WitTypeDef {
                name,
                doc,
                kind: WitTypeDefKind::Resource(WitResourceDef {
                    constructor: None,
                    methods: Vec::new(),
                    statics: Vec::new(),
                }),
            });
        }

        self.expect(&WitTokenKind::LBrace)?;

        let mut resource = WitResourceDef {
            constructor: None,
            methods: Vec::new(),
            statics: Vec::new(),
        };

        loop {
            let item_doc = self.take_doc();
            match self.peek().clone() {
                WitTokenKind::RBrace => {
                    self.advance();
                    break;
                }
                WitTokenKind::Constructor => {
                    self.advance();
                    self.expect(&WitTokenKind::LParen)?;
                    let params = self.parse_param_list()?;
                    self.expect(&WitTokenKind::RParen)?;
                    self.expect(&WitTokenKind::Semicolon)?;
                    resource.constructor = Some(WitFuncDef {
                        name: "constructor".into(),
                        doc: item_doc,
                        params,
                        result: None,
                        is_static: false,
                        is_constructor: true,
                    });
                }
                WitTokenKind::Ident(_) => {
                    // Could be `name: func(...)` (method) or `name: static func(...)`
                    let func = self.parse_func_item(item_doc)?;
                    if func.is_static {
                        resource.statics.push(func);
                    } else {
                        resource.methods.push(func);
                    }
                }
                WitTokenKind::Eof => {
                    return Err(WitParseError {
                        message: "unexpected end of file in resource".into(),
                        offset: self.offset(),
                    });
                }
                other => {
                    return Err(WitParseError {
                        message: format!("unexpected token in resource: `{other}`"),
                        offset: self.offset(),
                    });
                }
            }
        }

        Ok(WitTypeDef {
            name,
            doc,
            kind: WitTypeDefKind::Resource(resource),
        })
    }

    /// Type alias: `type name = other-type;`
    fn parse_type_alias(&mut self, doc: Option<String>) -> Result<WitTypeDef, WitParseError> {
        self.expect(&WitTokenKind::Type)?;
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::Equals)?;
        let target = self.parse_type_ref()?;
        self.expect(&WitTokenKind::Semicolon)?;
        Ok(WitTypeDef {
            name,
            doc,
            kind: WitTypeDefKind::Alias(target),
        })
    }

    // ── Functions ──

    /// Parse a function item: `name: func(params) -> result;`
    fn parse_func_item(&mut self, doc: Option<String>) -> Result<WitFuncDef, WitParseError> {
        let name = self.expect_ident()?;
        self.expect(&WitTokenKind::Colon)?;

        let is_static = self.eat(&WitTokenKind::Static);

        let func = self.parse_func_body(name, doc)?;
        self.expect(&WitTokenKind::Semicolon)?;

        Ok(WitFuncDef { is_static, ..func })
    }

    /// Parse constructor func: `constructor(params);`
    fn parse_constructor_func(&mut self, doc: Option<String>) -> Result<WitFuncDef, WitParseError> {
        self.expect(&WitTokenKind::Constructor)?;
        self.expect(&WitTokenKind::LParen)?;
        let params = self.parse_param_list()?;
        self.expect(&WitTokenKind::RParen)?;
        self.expect(&WitTokenKind::Semicolon)?;
        Ok(WitFuncDef {
            name: "constructor".into(),
            doc,
            params,
            result: None,
            is_static: false,
            is_constructor: true,
        })
    }

    /// Parse `func(params) -> result` (does NOT consume trailing semicolon).
    fn parse_func_body(
        &mut self,
        name: String,
        doc: Option<String>,
    ) -> Result<WitFuncDef, WitParseError> {
        self.expect(&WitTokenKind::Func)?;
        self.expect(&WitTokenKind::LParen)?;
        let params = self.parse_param_list()?;
        self.expect(&WitTokenKind::RParen)?;

        let result = if self.eat(&WitTokenKind::Arrow) {
            Some(self.parse_type_ref()?)
        } else {
            None
        };

        Ok(WitFuncDef {
            name,
            doc,
            params,
            result,
            is_static: false,
            is_constructor: false,
        })
    }

    fn parse_param_list(&mut self) -> Result<Vec<WitParam>, WitParseError> {
        let mut params = Vec::new();
        if self.peek() == &WitTokenKind::RParen {
            return Ok(params);
        }
        loop {
            let name = self.expect_ident()?;
            self.expect(&WitTokenKind::Colon)?;
            let ty = self.parse_type_ref()?;
            params.push(WitParam { name, ty });
            if !self.eat(&WitTokenKind::Comma) {
                break;
            }
        }
        Ok(params)
    }

    // ── Type references ──

    /// W1.3 + W1.8: Parse a type reference.
    fn parse_type_ref(&mut self) -> Result<WitTypeRef, WitParseError> {
        match self.peek().clone() {
            // ── Primitives ──
            WitTokenKind::U8 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::U8))
            }
            WitTokenKind::U16 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::U16))
            }
            WitTokenKind::U32 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::U32))
            }
            WitTokenKind::U64 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::U64))
            }
            WitTokenKind::S8 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::S8))
            }
            WitTokenKind::S16 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::S16))
            }
            WitTokenKind::S32 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::S32))
            }
            WitTokenKind::S64 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::S64))
            }
            WitTokenKind::F32 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::F32))
            }
            WitTokenKind::F64 => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::F64))
            }
            WitTokenKind::Bool => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::Bool))
            }
            WitTokenKind::Char => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::Char))
            }
            WitTokenKind::StringKw => {
                self.advance();
                Ok(WitTypeRef::Primitive(WitPrimitive::String_))
            }

            // ── Generic built-ins ──
            WitTokenKind::List => {
                self.advance();
                self.expect(&WitTokenKind::LAngle)?;
                let inner = self.parse_type_ref()?;
                self.expect(&WitTokenKind::RAngle)?;
                Ok(WitTypeRef::List(Box::new(inner)))
            }
            WitTokenKind::Option_ => {
                self.advance();
                self.expect(&WitTokenKind::LAngle)?;
                let inner = self.parse_type_ref()?;
                self.expect(&WitTokenKind::RAngle)?;
                Ok(WitTypeRef::Option(Box::new(inner)))
            }
            WitTokenKind::Result_ => {
                self.advance();
                // result can be bare, or have type args
                if self.peek() != &WitTokenKind::LAngle {
                    return Ok(WitTypeRef::Result {
                        ok: None,
                        err: None,
                    });
                }
                self.advance(); // <
                // Could be `_` for no ok type
                let ok = if self.peek() == &WitTokenKind::Ident("_".into()) {
                    self.advance();
                    None
                } else {
                    Some(Box::new(self.parse_type_ref()?))
                };
                let err = if self.eat(&WitTokenKind::Comma) {
                    if self.peek() == &WitTokenKind::Ident("_".into()) {
                        self.advance();
                        None
                    } else {
                        Some(Box::new(self.parse_type_ref()?))
                    }
                } else {
                    None
                };
                self.expect(&WitTokenKind::RAngle)?;
                Ok(WitTypeRef::Result { ok, err })
            }
            WitTokenKind::Tuple_ => {
                self.advance();
                self.expect(&WitTokenKind::LAngle)?;
                let mut items = Vec::new();
                if self.peek() != &WitTokenKind::RAngle {
                    loop {
                        items.push(self.parse_type_ref()?);
                        if !self.eat(&WitTokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&WitTokenKind::RAngle)?;
                Ok(WitTypeRef::Tuple(items))
            }
            WitTokenKind::Own => {
                self.advance();
                self.expect(&WitTokenKind::LAngle)?;
                let inner = self.parse_type_ref()?;
                self.expect(&WitTokenKind::RAngle)?;
                Ok(WitTypeRef::Own(Box::new(inner)))
            }
            WitTokenKind::Borrow => {
                self.advance();
                self.expect(&WitTokenKind::LAngle)?;
                let inner = self.parse_type_ref()?;
                self.expect(&WitTokenKind::RAngle)?;
                Ok(WitTypeRef::Borrow(Box::new(inner)))
            }

            // ── Named type ──
            WitTokenKind::Ident(s) => {
                let name = s.clone();
                self.advance();
                Ok(WitTypeRef::Named(name))
            }

            other => Err(WitParseError {
                message: format!("expected type, found `{other}`"),
                offset: self.offset(),
            }),
        }
    }
}

/// Convenience: parse a WIT source string into a document.
pub fn parse_wit(source: &str) -> Result<WitDocument, WitParseError> {
    let tokens = tokenize_wit(source).map_err(|e| WitParseError {
        message: e.message,
        offset: e.offset,
    })?;
    WitParser::new(tokens).parse_document()
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── W1.2: Basic parsing ──

    #[test]
    fn w1_2_parse_empty_document() {
        let doc = parse_wit("").unwrap();
        assert!(doc.package.is_none());
        assert!(doc.interfaces.is_empty());
        assert!(doc.worlds.is_empty());
    }

    #[test]
    fn w1_2_parse_package_declaration() {
        let doc = parse_wit("package wasi:cli@0.2.0;").unwrap();
        let pkg = doc.package.unwrap();
        assert_eq!(pkg.namespace, "wasi");
        assert_eq!(pkg.name, "cli");
        assert_eq!(pkg.version, Some("0.2.0".into()));
    }

    #[test]
    fn w1_2_parse_empty_interface() {
        let doc = parse_wit("interface test {}").unwrap();
        assert_eq!(doc.interfaces.len(), 1);
        assert_eq!(doc.interfaces[0].name, "test");
        assert!(doc.interfaces[0].items.is_empty());
    }

    #[test]
    fn w1_2_parse_interface_with_function() {
        let src = r#"
interface demo {
    greet: func(name: string) -> string;
}
"#;
        let doc = parse_wit(src).unwrap();
        assert_eq!(doc.interfaces.len(), 1);
        let iface = &doc.interfaces[0];
        assert_eq!(iface.name, "demo");
        assert_eq!(iface.items.len(), 1);
        if let WitInterfaceItem::Func(f) = &iface.items[0] {
            assert_eq!(f.name, "greet");
            assert_eq!(f.params.len(), 1);
            assert_eq!(f.params[0].name, "name");
            assert_eq!(f.params[0].ty, WitTypeRef::Primitive(WitPrimitive::String_));
            assert_eq!(f.result, Some(WitTypeRef::Primitive(WitPrimitive::String_)));
        } else {
            panic!("expected Func");
        }
    }

    #[test]
    fn w1_2_parse_world_with_imports_exports() {
        let src = r#"
package wasi:cli@0.2.0;

world command {
    import wasi:io/streams@0.2.0;
    export run: func() -> result;
}
"#;
        let doc = parse_wit(src).unwrap();
        let pkg = doc.package.unwrap();
        assert_eq!(pkg.to_string(), "wasi:cli@0.2.0");

        assert_eq!(doc.worlds.len(), 1);
        let world = &doc.worlds[0];
        assert_eq!(world.name, "command");
        assert_eq!(world.items.len(), 2);
    }

    // ── W1.3: Type system ──

    #[test]
    fn w1_3_parse_all_15_primitive_types() {
        let src = r#"
interface types {
    t1: func() -> u8;
    t2: func() -> u16;
    t3: func() -> u32;
    t4: func() -> u64;
    t5: func() -> s8;
    t6: func() -> s16;
    t7: func() -> s32;
    t8: func() -> s64;
    t9: func() -> f32;
    t10: func() -> f64;
    t11: func() -> bool;
    t12: func() -> char;
    t13: func() -> string;
    t14: func() -> list<u8>;
    t15: func() -> option<string>;
}
"#;
        let doc = parse_wit(src).unwrap();
        assert_eq!(doc.interfaces[0].items.len(), 15);
        // Verify first and last
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[0] {
            assert_eq!(f.result, Some(WitTypeRef::Primitive(WitPrimitive::U8)));
        }
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[14] {
            assert_eq!(
                f.result,
                Some(WitTypeRef::Option(Box::new(WitTypeRef::Primitive(
                    WitPrimitive::String_
                ))))
            );
        }
    }

    // ── W1.4: Record types ──

    #[test]
    fn w1_4_parse_record() {
        let src = r#"
interface geo {
    record point {
        x: f64,
        y: f64,
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::TypeDef(td) = &doc.interfaces[0].items[0] {
            assert_eq!(td.name, "point");
            if let WitTypeDefKind::Record(fields) = &td.kind {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name, "x");
                assert_eq!(fields[0].ty, WitTypeRef::Primitive(WitPrimitive::F64));
                assert_eq!(fields[1].name, "y");
            } else {
                panic!("expected Record");
            }
        } else {
            panic!("expected TypeDef");
        }
    }

    #[test]
    fn w1_4_record_fields_accessible() {
        let src = r#"
interface fs {
    record filestat {
        device: u64,
        inode: u64,
        filetype: u8,
        nlink: u64,
        size: u64,
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::TypeDef(td) = &doc.interfaces[0].items[0] {
            if let WitTypeDefKind::Record(fields) = &td.kind {
                assert_eq!(fields.len(), 5);
                assert_eq!(fields[4].name, "size");
                assert_eq!(fields[4].ty, WitTypeRef::Primitive(WitPrimitive::U64));
            } else {
                panic!("expected Record");
            }
        }
    }

    // ── W1.5: Variant types ──

    #[test]
    fn w1_5_parse_variant() {
        let src = r#"
interface errors {
    variant error {
        timeout,
        refused(string),
        other(u32),
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::TypeDef(td) = &doc.interfaces[0].items[0] {
            assert_eq!(td.name, "error");
            if let WitTypeDefKind::Variant(cases) = &td.kind {
                assert_eq!(cases.len(), 3);
                assert_eq!(cases[0].name, "timeout");
                assert!(cases[0].ty.is_none());
                assert_eq!(cases[1].name, "refused");
                assert_eq!(
                    cases[1].ty,
                    Some(WitTypeRef::Primitive(WitPrimitive::String_))
                );
                assert_eq!(cases[2].name, "other");
                assert_eq!(cases[2].ty, Some(WitTypeRef::Primitive(WitPrimitive::U32)));
            } else {
                panic!("expected Variant");
            }
        }
    }

    #[test]
    fn w1_5_parse_enum() {
        let src = r#"
interface http {
    enum method {
        get,
        post,
        put,
        delete,
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::TypeDef(td) = &doc.interfaces[0].items[0] {
            if let WitTypeDefKind::Enum(cases) = &td.kind {
                assert_eq!(cases.len(), 4);
                assert_eq!(cases[0].name, "get");
                assert_eq!(cases[3].name, "delete");
            } else {
                panic!("expected Enum");
            }
        }
    }

    // ── W1.6: Flags types ──

    #[test]
    fn w1_6_parse_flags() {
        let src = r#"
interface fs {
    flags permissions {
        read,
        write,
        exec,
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::TypeDef(td) = &doc.interfaces[0].items[0] {
            assert_eq!(td.name, "permissions");
            if let WitTypeDefKind::Flags(flags) = &td.kind {
                assert_eq!(flags.len(), 3);
                assert_eq!(flags[0], "read");
                assert_eq!(flags[1], "write");
                assert_eq!(flags[2], "exec");
            } else {
                panic!("expected Flags");
            }
        }
    }

    // ── W1.7: Resource types ──

    #[test]
    fn w1_7_parse_resource_with_methods() {
        let src = r#"
interface fs {
    resource file {
        constructor(path: string);
        read: func(len: u64) -> list<u8>;
        write: func(data: list<u8>) -> u64;
        close: func();
        open: static func(path: string) -> file;
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::TypeDef(td) = &doc.interfaces[0].items[0] {
            assert_eq!(td.name, "file");
            if let WitTypeDefKind::Resource(res) = &td.kind {
                assert!(res.constructor.is_some());
                let ctor = res.constructor.as_ref().unwrap();
                assert!(ctor.is_constructor);
                assert_eq!(ctor.params.len(), 1);
                assert_eq!(ctor.params[0].name, "path");

                assert_eq!(res.methods.len(), 3); // read, write, close
                assert_eq!(res.methods[0].name, "read");
                assert_eq!(res.methods[1].name, "write");
                assert_eq!(res.methods[2].name, "close");

                assert_eq!(res.statics.len(), 1); // open
                assert_eq!(res.statics[0].name, "open");
                assert!(res.statics[0].is_static);
            } else {
                panic!("expected Resource");
            }
        }
    }

    #[test]
    fn w1_7_opaque_resource() {
        let src = r#"
interface streams {
    resource input-stream;
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::TypeDef(td) = &doc.interfaces[0].items[0] {
            assert_eq!(td.name, "input-stream");
            if let WitTypeDefKind::Resource(res) = &td.kind {
                assert!(res.constructor.is_none());
                assert!(res.methods.is_empty());
                assert!(res.statics.is_empty());
            } else {
                panic!("expected Resource");
            }
        }
    }

    // ── W1.8: Tuple/Option/Result ──

    #[test]
    fn w1_8_parse_option_type() {
        let src = r#"
interface test {
    get: func() -> option<string>;
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[0] {
            assert_eq!(
                f.result,
                Some(WitTypeRef::Option(Box::new(WitTypeRef::Primitive(
                    WitPrimitive::String_
                ))))
            );
        }
    }

    #[test]
    fn w1_8_parse_result_type() {
        let src = r#"
interface test {
    try-read: func() -> result<list<u8>, string>;
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[0] {
            if let Some(WitTypeRef::Result { ok, err }) = &f.result {
                assert!(ok.is_some());
                assert!(err.is_some());
                assert_eq!(
                    **err.as_ref().unwrap(),
                    WitTypeRef::Primitive(WitPrimitive::String_)
                );
            } else {
                panic!("expected result type");
            }
        }
    }

    #[test]
    fn w1_8_parse_bare_result() {
        let src = r#"
interface test {
    run: func() -> result;
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[0] {
            assert_eq!(
                f.result,
                Some(WitTypeRef::Result {
                    ok: None,
                    err: None
                })
            );
        }
    }

    #[test]
    fn w1_8_parse_tuple_type() {
        let src = r#"
interface test {
    coords: func() -> tuple<f64, f64, f64>;
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[0] {
            if let Some(WitTypeRef::Tuple(items)) = &f.result {
                assert_eq!(items.len(), 3);
                assert!(
                    items
                        .iter()
                        .all(|t| *t == WitTypeRef::Primitive(WitPrimitive::F64))
                );
            } else {
                panic!("expected tuple");
            }
        }
    }

    #[test]
    fn w1_8_own_and_borrow() {
        let src = r#"
interface test {
    take: func(f: own<file>) -> bool;
    peek: func(f: borrow<file>) -> string;
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[0] {
            assert_eq!(
                f.params[0].ty,
                WitTypeRef::Own(Box::new(WitTypeRef::Named("file".into())))
            );
        }
        if let WitInterfaceItem::Func(f) = &doc.interfaces[0].items[1] {
            assert_eq!(
                f.params[0].ty,
                WitTypeRef::Borrow(Box::new(WitTypeRef::Named("file".into())))
            );
        }
    }

    // ── W1.9: Use imports ──

    #[test]
    fn w1_9_parse_use_single_name() {
        let src = r#"
interface consumer {
    use wasi:filesystem/types.{descriptor};
    open: func(d: descriptor) -> bool;
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::Use(u) = &doc.interfaces[0].items[0] {
            assert_eq!(u.from.path, "wasi:filesystem/types");
            assert_eq!(u.names.len(), 1);
            assert_eq!(u.names[0].name, "descriptor");
            assert!(u.names[0].alias.is_none());
        } else {
            panic!("expected Use");
        }
    }

    #[test]
    fn w1_9_parse_use_multiple_names_with_alias() {
        let src = r#"
interface consumer {
    use wasi:io/streams.{input-stream, output-stream as out};
}
"#;
        let doc = parse_wit(src).unwrap();
        if let WitInterfaceItem::Use(u) = &doc.interfaces[0].items[0] {
            assert_eq!(u.names.len(), 2);
            assert_eq!(u.names[0].name, "input-stream");
            assert!(u.names[0].alias.is_none());
            assert_eq!(u.names[1].name, "output-stream");
            assert_eq!(u.names[1].alias, Some("out".into()));
        }
    }

    // ── W1.10: Comprehensive WIT parsing tests ──

    #[test]
    fn w1_10_parse_wasi_cli_world() {
        let src = r#"
package wasi:cli@0.2.0;

world command {
    import wasi:io/streams@0.2.0;
    import wasi:filesystem/types@0.2.0;
    import wasi:cli/stdin@0.2.0;
    import wasi:cli/stdout@0.2.0;
    import wasi:clocks/monotonic-clock@0.2.0;
    import wasi:random/random@0.2.0;
    export run: func() -> result;
}
"#;
        let doc = parse_wit(src).unwrap();
        assert_eq!(doc.package.as_ref().unwrap().namespace, "wasi");
        assert_eq!(doc.worlds.len(), 1);
        let world = &doc.worlds[0];
        assert_eq!(world.name, "command");
        // 6 imports + 1 export = 7 items
        assert_eq!(world.items.len(), 7);
    }

    #[test]
    fn w1_10_parse_wasi_http_world() {
        let src = r#"
package wasi:http@0.2.0;

interface types {
    record request {
        method: string,
        url: string,
        headers: list<tuple<string, string>>,
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
}

interface outgoing-handler {
    use types.{request, response, error};
    handle: func(req: request) -> result<response, error>;
}

interface incoming-handler {
    use types.{request, response};
    handle: func(req: request) -> response;
}

world proxy {
    import outgoing-handler;
    export incoming-handler;
}
"#;
        let doc = parse_wit(src).unwrap();
        assert_eq!(doc.interfaces.len(), 3);
        assert_eq!(doc.worlds.len(), 1);

        // Check types interface
        let types_iface = &doc.interfaces[0];
        assert_eq!(types_iface.name, "types");
        assert_eq!(types_iface.items.len(), 3); // request, response, error

        // Check outgoing-handler has use + func
        let out_iface = &doc.interfaces[1];
        assert_eq!(out_iface.name, "outgoing-handler");
        assert_eq!(out_iface.items.len(), 2); // use + handle

        // Check world
        let world = &doc.worlds[0];
        assert_eq!(world.name, "proxy");
        assert_eq!(world.items.len(), 2);
    }

    #[test]
    fn w1_10_parse_wasi_filesystem_interface() {
        let src = r#"
interface filesystem {
    flags path-flags {
        symlink-follow,
        create,
        exclusive,
        truncate,
    }

    enum filetype {
        unknown,
        block-device,
        character-device,
        directory,
        regular-file,
        symbolic-link,
    }

    resource descriptor {
        constructor(path: string);
        read-via-stream: func(offset: u64) -> result<own<input-stream>, error>;
        write-via-stream: func(offset: u64) -> result<own<output-stream>, error>;
        stat: func() -> result<filestat, error>;
    }

    record filestat {
        device: u64,
        inode: u64,
        filetype: filetype,
        size: u64,
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        let fs = &doc.interfaces[0];
        assert_eq!(fs.name, "filesystem");
        assert_eq!(fs.items.len(), 4); // flags + enum + resource + record
    }

    #[test]
    fn w1_10_parse_wasi_sockets_interface() {
        let src = r#"
interface sockets {
    resource tcp-socket {
        constructor();
        start-connect: func(addr: string, port: u16) -> result;
        finish-connect: func() -> result;
        send: func(data: list<u8>) -> result<u64, string>;
        receive: func(len: u64) -> result<list<u8>, string>;
        shutdown: func() -> result;
    }
}
"#;
        let doc = parse_wit(src).unwrap();
        let iface = &doc.interfaces[0];
        if let WitInterfaceItem::TypeDef(td) = &iface.items[0] {
            if let WitTypeDefKind::Resource(res) = &td.kind {
                assert!(res.constructor.is_some());
                assert_eq!(res.methods.len(), 5);
            }
        }
    }

    #[test]
    fn w1_10_type_ref_display() {
        assert_eq!(WitTypeRef::Primitive(WitPrimitive::U32).to_string(), "u32");
        assert_eq!(
            WitTypeRef::List(Box::new(WitTypeRef::Primitive(WitPrimitive::U8))).to_string(),
            "list<u8>"
        );
        assert_eq!(
            WitTypeRef::Option(Box::new(WitTypeRef::Primitive(WitPrimitive::String_))).to_string(),
            "option<string>"
        );
        let result = WitTypeRef::Result {
            ok: Some(Box::new(WitTypeRef::Primitive(WitPrimitive::U32))),
            err: Some(Box::new(WitTypeRef::Primitive(WitPrimitive::String_))),
        };
        assert_eq!(result.to_string(), "result<u32, string>");
        let tuple = WitTypeRef::Tuple(vec![
            WitTypeRef::Primitive(WitPrimitive::F64),
            WitTypeRef::Primitive(WitPrimitive::F64),
        ]);
        assert_eq!(tuple.to_string(), "tuple<f64, f64>");
        assert_eq!(
            WitTypeRef::Own(Box::new(WitTypeRef::Named("file".into()))).to_string(),
            "own<file>"
        );
        assert_eq!(
            WitTypeRef::Borrow(Box::new(WitTypeRef::Named("file".into()))).to_string(),
            "borrow<file>"
        );
    }
}

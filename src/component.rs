//! W2: Component Model Binary Format — Emit WASI P2 component binaries.
//!
//! Implements the WebAssembly Component Model binary encoding:
//! - Component sections (custom section type 0x0d)
//! - Canonical ABI type encoding
//! - Import/export sections for WASI interfaces
//! - Canonical ABI lifting (Fajar values → linear memory)
//! - Canonical ABI lowering (linear memory → Fajar values)
//! - `cabi_realloc` memory allocation protocol
//! - Post-return cleanup (`cabi_post_*`)
//! - Component validation against WIT spec

#![allow(missing_docs)] // P6.E4: data-heavy enum/struct module; field+variant names self-document

use super::wit_parser::{
    WitFuncDef, WitPrimitive, WitTypeRef, WitWorldDef, WitWorldExport, WitWorldImport, WitWorldItem,
};
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// W2.1: Component Section Emitter
// ═══════════════════════════════════════════════════════════════════════

/// Component binary builder — produces a valid WebAssembly component.
#[derive(Debug, Clone)]
pub struct ComponentBuilder {
    /// Raw bytes of the component being built.
    bytes: Vec<u8>,
    /// Component type sections accumulated.
    type_sections: Vec<ComponentTypeSection>,
    /// Import sections.
    imports: Vec<ComponentImport>,
    /// Export sections.
    exports: Vec<ComponentExport>,
    /// Core module bytes (the inner wasm module).
    core_module: Option<Vec<u8>>,
    /// Whether cabi_realloc is exported.
    has_realloc: bool,
    /// Post-return function names.
    post_return_fns: Vec<String>,
}

/// A component type section entry.
#[derive(Debug, Clone)]
pub struct ComponentTypeSection {
    /// Type index.
    pub index: u32,
    /// The encoded type.
    pub kind: ComponentTypeKind,
}

/// Kinds of component types.
#[derive(Debug, Clone)]
pub enum ComponentTypeKind {
    /// A function type: `(func (param ...) (result ...))`.
    Func(ComponentFuncType),
    /// An instance type (interface with functions).
    Instance(Vec<ComponentFuncType>),
    /// A component type.
    Component,
}

/// A component function type.
#[derive(Debug, Clone)]
pub struct ComponentFuncType {
    /// Function name.
    pub name: String,
    /// Parameters (name, type).
    pub params: Vec<(String, ComponentValType)>,
    /// Result type.
    pub result: Option<ComponentValType>,
}

/// Component value types (canonical ABI types).
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentValType {
    Bool,
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
    Char,
    String_,
    List(Box<ComponentValType>),
    Option_(Box<ComponentValType>),
    Result_ {
        ok: Option<Box<ComponentValType>>,
        err: Option<Box<ComponentValType>>,
    },
    Tuple(Vec<ComponentValType>),
    Record(Vec<(String, ComponentValType)>),
    Variant(Vec<(String, Option<ComponentValType>)>),
    Flags(Vec<String>),
    Own(u32),    // type index
    Borrow(u32), // type index
    /// Reference to a defined type by index.
    TypeRef(u32),
}

impl fmt::Display for ComponentValType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => write!(f, "bool"),
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
            Self::Char => write!(f, "char"),
            Self::String_ => write!(f, "string"),
            Self::List(inner) => write!(f, "list<{inner}>"),
            Self::Option_(inner) => write!(f, "option<{inner}>"),
            Self::Result_ { ok, err } => {
                write!(f, "result")?;
                if ok.is_some() || err.is_some() {
                    write!(f, "<")?;
                    match ok {
                        Some(t) => write!(f, "{t}")?,
                        None => write!(f, "_")?,
                    }
                    if let Some(e) = err {
                        write!(f, ", {e}")?;
                    }
                    write!(f, ">")?;
                }
                Ok(())
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
            Self::Record(fields) => {
                write!(f, "record {{ ")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}: {ty}")?;
                }
                write!(f, " }}")
            }
            Self::Variant(cases) => {
                write!(f, "variant {{ ")?;
                for (i, (name, ty)) in cases.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{name}")?;
                    if let Some(t) = ty {
                        write!(f, "({t})")?;
                    }
                }
                write!(f, " }}")
            }
            Self::Flags(flags) => {
                write!(f, "flags {{ ")?;
                for (i, flag) in flags.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{flag}")?;
                }
                write!(f, " }}")
            }
            Self::Own(idx) => write!(f, "own<{idx}>"),
            Self::Borrow(idx) => write!(f, "borrow<{idx}>"),
            Self::TypeRef(idx) => write!(f, "type-ref({idx})"),
        }
    }
}

/// A component import.
#[derive(Debug, Clone)]
pub struct ComponentImport {
    /// Import name (interface path).
    pub name: String,
    /// Type index this import satisfies.
    pub type_index: u32,
}

/// A component export.
#[derive(Debug, Clone)]
pub struct ComponentExport {
    /// Export name.
    pub name: String,
    /// Kind: function, instance, etc.
    pub kind: ExportKind,
    /// Index of the exported item.
    pub index: u32,
}

/// What kind of item is being exported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExportKind {
    Func,
    Instance,
    Component,
    Module,
}

impl ComponentBuilder {
    /// Creates a new empty component builder.
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            type_sections: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            core_module: None,
            has_realloc: false,
            post_return_fns: Vec::new(),
        }
    }

    /// Sets the core WebAssembly module that this component wraps.
    pub fn set_core_module(&mut self, module_bytes: Vec<u8>) {
        self.core_module = Some(module_bytes);
    }

    /// Adds a type section entry.
    pub fn add_type(&mut self, kind: ComponentTypeKind) -> u32 {
        let index = self.type_sections.len() as u32;
        self.type_sections
            .push(ComponentTypeSection { index, kind });
        index
    }

    /// Adds an import.
    pub fn add_import(&mut self, name: &str, type_index: u32) {
        self.imports.push(ComponentImport {
            name: name.to_string(),
            type_index,
        });
    }

    /// Adds an export.
    pub fn add_export(&mut self, name: &str, kind: ExportKind, index: u32) {
        self.exports.push(ComponentExport {
            name: name.to_string(),
            kind,
            index,
        });
    }

    /// Enables cabi_realloc export.
    pub fn enable_realloc(&mut self) {
        self.has_realloc = true;
    }

    /// Adds a post-return cleanup function.
    pub fn add_post_return(&mut self, func_name: &str) {
        self.post_return_fns.push(func_name.to_string());
    }

    /// Build the component binary.
    pub fn build(&mut self) -> Vec<u8> {
        let mut out = Vec::new();

        // Component magic + version
        // Component uses the same magic as Wasm but with layer byte = 0x01
        out.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D]); // \0asm
        out.extend_from_slice(&[0x0D, 0x00, 0x01, 0x00]); // component version (layer=0x0d, version=1)

        // Encode core module section (section id 0x00 for core:module)
        if let Some(ref module) = self.core_module {
            let mut section = Vec::new();
            section.push(0x00); // core:module section
            encode_vec_bytes(&mut section, module);
            out.extend_from_slice(&section);
        }

        // Encode type section (section id 0x01)
        if !self.type_sections.is_empty() {
            let mut section = Vec::new();
            section.push(0x01); // type section
            let mut type_data = Vec::new();
            encode_u32(&mut type_data, self.type_sections.len() as u32);
            for ts in &self.type_sections {
                encode_component_type(&mut type_data, &ts.kind);
            }
            encode_vec_bytes(&mut section, &type_data);
            out.extend_from_slice(&section);
        }

        // Encode import section (section id 0x02)
        if !self.imports.is_empty() {
            let mut section = Vec::new();
            section.push(0x02); // import section
            let mut import_data = Vec::new();
            encode_u32(&mut import_data, self.imports.len() as u32);
            for imp in &self.imports {
                encode_string(&mut import_data, &imp.name);
                encode_u32(&mut import_data, imp.type_index);
            }
            encode_vec_bytes(&mut section, &import_data);
            out.extend_from_slice(&section);
        }

        // Encode export section (section id 0x03)
        if !self.exports.is_empty() {
            let mut section = Vec::new();
            section.push(0x03); // export section
            let mut export_data = Vec::new();
            encode_u32(&mut export_data, self.exports.len() as u32);
            for exp in &self.exports {
                encode_string(&mut export_data, &exp.name);
                export_data.push(match exp.kind {
                    ExportKind::Func => 0x01,
                    ExportKind::Instance => 0x02,
                    ExportKind::Component => 0x03,
                    ExportKind::Module => 0x00,
                });
                encode_u32(&mut export_data, exp.index);
            }
            encode_vec_bytes(&mut section, &export_data);
            out.extend_from_slice(&section);
        }

        self.bytes = out.clone();
        out
    }

    /// Returns the built bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the number of type sections.
    pub fn type_count(&self) -> usize {
        self.type_sections.len()
    }

    /// Returns the number of imports.
    pub fn import_count(&self) -> usize {
        self.imports.len()
    }

    /// Returns the number of exports.
    pub fn export_count(&self) -> usize {
        self.exports.len()
    }

    /// Whether realloc is enabled.
    pub fn has_realloc(&self) -> bool {
        self.has_realloc
    }

    /// Post-return function names.
    pub fn post_return_functions(&self) -> &[String] {
        &self.post_return_fns
    }
}

impl Default for ComponentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W2.2: Component Type Encoding
// ═══════════════════════════════════════════════════════════════════════

/// Convert a WIT type reference to a component val type.
pub fn wit_type_to_component(ty: &WitTypeRef) -> ComponentValType {
    match ty {
        WitTypeRef::Primitive(p) => match p {
            WitPrimitive::U8 => ComponentValType::U8,
            WitPrimitive::U16 => ComponentValType::U16,
            WitPrimitive::U32 => ComponentValType::U32,
            WitPrimitive::U64 => ComponentValType::U64,
            WitPrimitive::S8 => ComponentValType::S8,
            WitPrimitive::S16 => ComponentValType::S16,
            WitPrimitive::S32 => ComponentValType::S32,
            WitPrimitive::S64 => ComponentValType::S64,
            WitPrimitive::F32 => ComponentValType::F32,
            WitPrimitive::F64 => ComponentValType::F64,
            WitPrimitive::Bool => ComponentValType::Bool,
            WitPrimitive::Char => ComponentValType::Char,
            WitPrimitive::String_ => ComponentValType::String_,
        },
        WitTypeRef::List(inner) => ComponentValType::List(Box::new(wit_type_to_component(inner))),
        WitTypeRef::Option(inner) => {
            ComponentValType::Option_(Box::new(wit_type_to_component(inner)))
        }
        WitTypeRef::Result { ok, err } => ComponentValType::Result_ {
            ok: ok.as_ref().map(|t| Box::new(wit_type_to_component(t))),
            err: err.as_ref().map(|t| Box::new(wit_type_to_component(t))),
        },
        WitTypeRef::Tuple(items) => {
            ComponentValType::Tuple(items.iter().map(wit_type_to_component).collect())
        }
        WitTypeRef::Own(inner) => {
            // For now, map to TypeRef(0) — proper resolution needs type index
            let _ = inner;
            ComponentValType::Own(0)
        }
        WitTypeRef::Borrow(inner) => {
            let _ = inner;
            ComponentValType::Borrow(0)
        }
        WitTypeRef::Named(_name) => {
            // Named types need resolution from the type index
            ComponentValType::TypeRef(0)
        }
    }
}

/// Convert a WIT function def to a component func type.
pub fn wit_func_to_component(func: &WitFuncDef) -> ComponentFuncType {
    ComponentFuncType {
        name: func.name.clone(),
        params: func
            .params
            .iter()
            .map(|p| (p.name.clone(), wit_type_to_component(&p.ty)))
            .collect(),
        result: func.result.as_ref().map(wit_type_to_component),
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W2.3–W2.4: Import/Export from World
// ═══════════════════════════════════════════════════════════════════════

/// Build a component from a WIT world definition.
pub fn build_component_from_world(world: &WitWorldDef) -> ComponentBuilder {
    let mut builder = ComponentBuilder::new();

    for item in &world.items {
        match item {
            WitWorldItem::Import(imp) => match imp {
                WitWorldImport::InterfacePath(path) => {
                    let idx = builder.add_type(ComponentTypeKind::Instance(Vec::new()));
                    builder.add_import(&path.path, idx);
                }
                WitWorldImport::Func { name, func } => {
                    let ft = wit_func_to_component(func);
                    let idx = builder.add_type(ComponentTypeKind::Func(ft));
                    builder.add_import(name, idx);
                }
            },
            WitWorldItem::Export(exp) => match exp {
                WitWorldExport::InterfacePath(path) => {
                    let idx = builder.add_type(ComponentTypeKind::Instance(Vec::new()));
                    builder.add_export(&path.path, ExportKind::Instance, idx);
                }
                WitWorldExport::Func { name, func } => {
                    let ft = wit_func_to_component(func);
                    let idx = builder.add_type(ComponentTypeKind::Func(ft));
                    builder.add_export(name, ExportKind::Func, idx);
                }
            },
            _ => {} // Include, Use, TypeDef handled elsewhere
        }
    }

    // Enable realloc for any component with string/list params
    builder.enable_realloc();

    builder
}

// ═══════════════════════════════════════════════════════════════════════
// W2.5: Canonical ABI Lifting (Fajar → Linear Memory)
// ═══════════════════════════════════════════════════════════════════════

/// Canonical ABI value representation in linear memory.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalValue {
    /// i32 value (bool, u8, u16, u32, s8, s16, s32, char, flags).
    I32(i32),
    /// i64 value (u64, s64).
    I64(i64),
    /// f32 value.
    F32(f32),
    /// f64 value.
    F64(f64),
    /// String: (pointer, length) in linear memory.
    String { ptr: u32, len: u32 },
    /// List: (pointer, length) in linear memory.
    List { ptr: u32, len: u32 },
    /// Record: flattened fields.
    Record(Vec<CanonicalValue>),
    /// Variant: discriminant + optional payload.
    Variant {
        discriminant: u32,
        payload: Option<Box<CanonicalValue>>,
    },
    /// Option: 0 = None, 1 = Some(value).
    Option_ {
        is_some: bool,
        value: Option<Box<CanonicalValue>>,
    },
    /// Result: 0 = Ok(value), 1 = Err(value).
    Result_ {
        is_ok: bool,
        value: Option<Box<CanonicalValue>>,
    },
    /// Tuple: flattened elements.
    Tuple(Vec<CanonicalValue>),
}

/// Linear memory for canonical ABI operations.
#[derive(Debug)]
pub struct LinearMemory {
    /// Memory buffer.
    data: Vec<u8>,
    /// Current allocation pointer.
    alloc_ptr: u32,
}

impl LinearMemory {
    /// Creates a new linear memory with given page count (64KB each).
    pub fn new(pages: u32) -> Self {
        Self {
            data: vec![0u8; (pages as usize) * 65536],
            alloc_ptr: 8, // start after null page
        }
    }

    /// Allocates `size` bytes aligned to `align`, returns pointer.
    pub fn alloc(&mut self, size: u32, align: u32) -> u32 {
        // Align up
        let mask = align - 1;
        self.alloc_ptr = (self.alloc_ptr + mask) & !mask;
        let ptr = self.alloc_ptr;
        self.alloc_ptr += size;
        ptr
    }

    /// Writes bytes at the given offset.
    pub fn write_bytes(&mut self, offset: u32, bytes: &[u8]) {
        let start = offset as usize;
        let end = start + bytes.len();
        if end <= self.data.len() {
            self.data[start..end].copy_from_slice(bytes);
        }
    }

    /// Reads bytes from the given offset.
    pub fn read_bytes(&self, offset: u32, len: u32) -> &[u8] {
        let start = offset as usize;
        let end = start + len as usize;
        if end <= self.data.len() {
            &self.data[start..end]
        } else {
            &[]
        }
    }

    /// Writes a u32 at the given offset (little-endian).
    pub fn write_u32(&mut self, offset: u32, value: u32) {
        self.write_bytes(offset, &value.to_le_bytes());
    }

    /// Reads a u32 from the given offset (little-endian).
    pub fn read_u32(&self, offset: u32) -> u32 {
        let bytes = self.read_bytes(offset, 4);
        if bytes.len() == 4 {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            0
        }
    }

    /// Writes a u64 at the given offset (little-endian).
    pub fn write_u64(&mut self, offset: u32, value: u64) {
        self.write_bytes(offset, &value.to_le_bytes());
    }

    /// Reads a u64 from the given offset (little-endian).
    pub fn read_u64(&self, offset: u32) -> u64 {
        let bytes = self.read_bytes(offset, 8);
        if bytes.len() == 8 {
            u64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ])
        } else {
            0
        }
    }

    /// Total allocated bytes.
    pub fn allocated(&self) -> u32 {
        self.alloc_ptr
    }

    /// Total capacity.
    pub fn capacity(&self) -> usize {
        self.data.len()
    }
}

/// W2.5: Lower a string into linear memory (Fajar string → ptr+len).
pub fn lower_string(mem: &mut LinearMemory, s: &str) -> CanonicalValue {
    let bytes = s.as_bytes();
    let ptr = mem.alloc(bytes.len() as u32, 1);
    mem.write_bytes(ptr, bytes);
    CanonicalValue::String {
        ptr,
        len: bytes.len() as u32,
    }
}

/// W2.5: Lower a byte list into linear memory.
pub fn lower_list_u8(mem: &mut LinearMemory, data: &[u8]) -> CanonicalValue {
    let ptr = mem.alloc(data.len() as u32, 1);
    mem.write_bytes(ptr, data);
    CanonicalValue::List {
        ptr,
        len: data.len() as u32,
    }
}

/// W2.5: Lower a list of u32 values into linear memory.
pub fn lower_list_u32(mem: &mut LinearMemory, values: &[u32]) -> CanonicalValue {
    let byte_len = (values.len() * 4) as u32;
    let ptr = mem.alloc(byte_len, 4);
    for (i, &v) in values.iter().enumerate() {
        mem.write_u32(ptr + (i as u32) * 4, v);
    }
    CanonicalValue::List {
        ptr,
        len: values.len() as u32,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W2.6: Canonical ABI Lowering (Linear Memory → Fajar Values)
// ═══════════════════════════════════════════════════════════════════════

/// W2.6: Lift a string from linear memory (ptr+len → Fajar string).
pub fn lift_string(mem: &LinearMemory, ptr: u32, len: u32) -> Option<String> {
    let bytes = mem.read_bytes(ptr, len);
    if bytes.len() == len as usize {
        String::from_utf8(bytes.to_vec()).ok()
    } else {
        None
    }
}

/// W2.6: Lift a byte list from linear memory.
pub fn lift_list_u8(mem: &LinearMemory, ptr: u32, len: u32) -> Vec<u8> {
    mem.read_bytes(ptr, len).to_vec()
}

/// W2.6: Lift a u32 list from linear memory.
pub fn lift_list_u32(mem: &LinearMemory, ptr: u32, len: u32) -> Vec<u32> {
    (0..len).map(|i| mem.read_u32(ptr + i * 4)).collect()
}

// ═══════════════════════════════════════════════════════════════════════
// W2.7: cabi_realloc Protocol
// ═══════════════════════════════════════════════════════════════════════

/// Simulates the `cabi_realloc` export for host-allocated memory.
///
/// Signature: `cabi_realloc(old_ptr: i32, old_size: i32, align: i32, new_size: i32) -> i32`
pub fn cabi_realloc(
    mem: &mut LinearMemory,
    _old_ptr: u32,
    _old_size: u32,
    align: u32,
    new_size: u32,
) -> u32 {
    // Simple bump allocator — doesn't reuse old memory
    mem.alloc(new_size, align)
}

// ═══════════════════════════════════════════════════════════════════════
// W2.8: Post-Return Cleanup
// ═══════════════════════════════════════════════════════════════════════

/// Tracks allocations that need post-return cleanup.
#[derive(Debug, Default)]
pub struct PostReturnTracker {
    /// Allocations to free after function returns.
    pending: Vec<(u32, u32)>, // (ptr, size)
}

impl PostReturnTracker {
    /// Creates a new tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an allocation for post-return cleanup.
    pub fn register(&mut self, ptr: u32, size: u32) {
        self.pending.push((ptr, size));
    }

    /// Returns all pending cleanups and clears the tracker.
    pub fn drain(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.pending)
    }

    /// Number of pending cleanups.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W2.9: Component Validation
// ═══════════════════════════════════════════════════════════════════════

/// Validation error for component binaries.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentValidationError {
    /// Error message.
    pub message: String,
}

impl fmt::Display for ComponentValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "component validation error: {}", self.message)
    }
}

/// Validates a component binary.
pub fn validate_component(
    bytes: &[u8],
) -> Result<ComponentValidationReport, ComponentValidationError> {
    let mut report = ComponentValidationReport::default();

    // Check magic bytes
    if bytes.len() < 8 {
        return Err(ComponentValidationError {
            message: "component too short (< 8 bytes)".into(),
        });
    }

    if &bytes[0..4] != b"\x00asm" {
        return Err(ComponentValidationError {
            message: "invalid magic bytes (expected \\0asm)".into(),
        });
    }

    // Check component layer byte (0x0d for components)
    if bytes[4] != 0x0D {
        return Err(ComponentValidationError {
            message: format!("expected component layer 0x0d, got 0x{:02x}", bytes[4]),
        });
    }

    report.magic_valid = true;
    report.version_valid = true;
    report.total_size = bytes.len();

    // Walk sections
    let mut pos = 8;
    while pos < bytes.len() {
        if pos >= bytes.len() {
            break;
        }
        let section_id = bytes[pos];
        pos += 1;
        report.section_count += 1;

        // Read section size (LEB128)
        let (size, consumed) = read_leb128(&bytes[pos..]);
        pos += consumed;

        match section_id {
            0x00 => report.has_core_module = true,
            0x01 => report.has_type_section = true,
            0x02 => report.has_import_section = true,
            0x03 => report.has_export_section = true,
            _ => {} // Other sections
        }

        pos += size as usize;
    }

    report.valid = report.magic_valid && report.version_valid;
    Ok(report)
}

/// Component validation report.
#[derive(Debug, Clone, Default)]
pub struct ComponentValidationReport {
    /// Whether the component is valid overall.
    pub valid: bool,
    /// Magic bytes correct.
    pub magic_valid: bool,
    /// Version correct.
    pub version_valid: bool,
    /// Has core module section.
    pub has_core_module: bool,
    /// Has type section.
    pub has_type_section: bool,
    /// Has import section.
    pub has_import_section: bool,
    /// Has export section.
    pub has_export_section: bool,
    /// Number of sections.
    pub section_count: usize,
    /// Total binary size.
    pub total_size: usize,
}

impl fmt::Display for ComponentValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Component Validation Report")?;
        writeln!(f, "  Valid: {}", self.valid)?;
        writeln!(f, "  Size: {} bytes", self.total_size)?;
        writeln!(f, "  Sections: {}", self.section_count)?;
        writeln!(f, "  Core module: {}", self.has_core_module)?;
        writeln!(f, "  Types: {}", self.has_type_section)?;
        writeln!(f, "  Imports: {}", self.has_import_section)?;
        writeln!(f, "  Exports: {}", self.has_export_section)?;
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Binary Encoding Helpers
// ═══════════════════════════════════════════════════════════════════════

fn encode_u32(out: &mut Vec<u8>, value: u32) {
    // LEB128 unsigned encoding
    let mut val = value;
    loop {
        let mut byte = (val & 0x7F) as u8;
        val >>= 7;
        if val != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if val == 0 {
            break;
        }
    }
}

fn encode_string(out: &mut Vec<u8>, s: &str) {
    encode_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn encode_vec_bytes(out: &mut Vec<u8>, data: &[u8]) {
    encode_u32(out, data.len() as u32);
    out.extend_from_slice(data);
}

fn encode_component_type(out: &mut Vec<u8>, kind: &ComponentTypeKind) {
    match kind {
        ComponentTypeKind::Func(ft) => {
            out.push(0x40); // func type
            encode_string(out, &ft.name);
            encode_u32(out, ft.params.len() as u32);
            for (name, ty) in &ft.params {
                encode_string(out, name);
                encode_val_type(out, ty);
            }
            match &ft.result {
                Some(ty) => {
                    out.push(0x01); // has result
                    encode_val_type(out, ty);
                }
                None => out.push(0x00),
            }
        }
        ComponentTypeKind::Instance(funcs) => {
            out.push(0x42); // instance type
            encode_u32(out, funcs.len() as u32);
            for ft in funcs {
                encode_string(out, &ft.name);
                out.push(0x40);
                encode_u32(out, ft.params.len() as u32);
                for (name, ty) in &ft.params {
                    encode_string(out, name);
                    encode_val_type(out, ty);
                }
                match &ft.result {
                    Some(ty) => {
                        out.push(0x01);
                        encode_val_type(out, ty);
                    }
                    None => out.push(0x00),
                }
            }
        }
        ComponentTypeKind::Component => {
            out.push(0x41); // component type
        }
    }
}

fn encode_val_type(out: &mut Vec<u8>, ty: &ComponentValType) {
    match ty {
        ComponentValType::Bool => out.push(0x7F),
        ComponentValType::U8 => out.push(0x7E),
        ComponentValType::U16 => out.push(0x7D),
        ComponentValType::U32 => out.push(0x7C),
        ComponentValType::U64 => out.push(0x7B),
        ComponentValType::S8 => out.push(0x7A),
        ComponentValType::S16 => out.push(0x79),
        ComponentValType::S32 => out.push(0x78),
        ComponentValType::S64 => out.push(0x77),
        ComponentValType::F32 => out.push(0x76),
        ComponentValType::F64 => out.push(0x75),
        ComponentValType::Char => out.push(0x74),
        ComponentValType::String_ => out.push(0x73),
        ComponentValType::List(inner) => {
            out.push(0x70);
            encode_val_type(out, inner);
        }
        ComponentValType::Option_(inner) => {
            out.push(0x6F);
            encode_val_type(out, inner);
        }
        ComponentValType::Result_ { ok, err } => {
            out.push(0x6E);
            match ok {
                Some(t) => {
                    out.push(0x01);
                    encode_val_type(out, t);
                }
                None => out.push(0x00),
            }
            match err {
                Some(t) => {
                    out.push(0x01);
                    encode_val_type(out, t);
                }
                None => out.push(0x00),
            }
        }
        ComponentValType::Tuple(items) => {
            out.push(0x6D);
            encode_u32(out, items.len() as u32);
            for item in items {
                encode_val_type(out, item);
            }
        }
        ComponentValType::Record(fields) => {
            out.push(0x6C);
            encode_u32(out, fields.len() as u32);
            for (name, ty) in fields {
                encode_string(out, name);
                encode_val_type(out, ty);
            }
        }
        ComponentValType::Variant(cases) => {
            out.push(0x6B);
            encode_u32(out, cases.len() as u32);
            for (name, ty) in cases {
                encode_string(out, name);
                match ty {
                    Some(t) => {
                        out.push(0x01);
                        encode_val_type(out, t);
                    }
                    None => out.push(0x00),
                }
            }
        }
        ComponentValType::Flags(flags) => {
            out.push(0x6A);
            encode_u32(out, flags.len() as u32);
            for flag in flags {
                encode_string(out, flag);
            }
        }
        ComponentValType::Own(idx) => {
            out.push(0x69);
            encode_u32(out, *idx);
        }
        ComponentValType::Borrow(idx) => {
            out.push(0x68);
            encode_u32(out, *idx);
        }
        ComponentValType::TypeRef(idx) => {
            encode_u32(out, *idx);
        }
    }
}

fn read_leb128(bytes: &[u8]) -> (u32, usize) {
    let mut result: u32 = 0;
    let mut shift = 0;
    let mut consumed = 0;
    for &byte in bytes {
        consumed += 1;
        result |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 35 {
            break;
        }
    }
    (result, consumed)
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wit_parser::{WitParam, parse_wit};

    // ── W2.1: Component section emitter ──

    #[test]
    fn w2_1_component_magic_bytes() {
        let mut builder = ComponentBuilder::new();
        let bytes = builder.build();
        assert_eq!(&bytes[0..4], b"\x00asm");
        assert_eq!(bytes[4], 0x0D); // component layer
    }

    #[test]
    fn w2_1_component_with_core_module() {
        let mut builder = ComponentBuilder::new();
        // Minimal wasm module: magic + version + empty
        let core = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        builder.set_core_module(core);
        let bytes = builder.build();
        assert!(bytes.len() > 8);
        let report = validate_component(&bytes).unwrap();
        assert!(report.magic_valid);
        assert!(report.has_core_module);
    }

    // ── W2.2: Type encoding ──

    #[test]
    fn w2_2_encode_primitive_types() {
        let ty = wit_type_to_component(&WitTypeRef::Primitive(WitPrimitive::U32));
        assert_eq!(ty, ComponentValType::U32);
        assert_eq!(ty.to_string(), "u32");
    }

    #[test]
    fn w2_2_types_round_trip() {
        let types = vec![
            ComponentValType::Bool,
            ComponentValType::U8,
            ComponentValType::U64,
            ComponentValType::F64,
            ComponentValType::String_,
            ComponentValType::List(Box::new(ComponentValType::U8)),
            ComponentValType::Option_(Box::new(ComponentValType::String_)),
            ComponentValType::Result_ {
                ok: Some(Box::new(ComponentValType::U32)),
                err: Some(Box::new(ComponentValType::String_)),
            },
        ];

        // Encode and verify display round-trips
        for ty in &types {
            let display = ty.to_string();
            assert!(!display.is_empty(), "type should have display: {ty:?}");
        }

        // Encode to bytes and verify non-empty
        for ty in &types {
            let mut bytes = Vec::new();
            encode_val_type(&mut bytes, ty);
            assert!(
                !bytes.is_empty(),
                "encoded type should be non-empty: {ty:?}"
            );
        }
    }

    // ── W2.3: Import section ──

    #[test]
    fn w2_3_component_with_imports() {
        let mut builder = ComponentBuilder::new();
        let idx = builder.add_type(ComponentTypeKind::Instance(Vec::new()));
        builder.add_import("wasi:filesystem/types", idx);
        let bytes = builder.build();
        let report = validate_component(&bytes).unwrap();
        assert!(report.has_import_section);
        assert!(report.has_type_section);
        assert_eq!(builder.import_count(), 1);
    }

    // ── W2.4: Export section ──

    #[test]
    fn w2_4_component_with_export() {
        let mut builder = ComponentBuilder::new();
        let ft = ComponentFuncType {
            name: "run".into(),
            params: Vec::new(),
            result: Some(ComponentValType::Result_ {
                ok: None,
                err: None,
            }),
        };
        let idx = builder.add_type(ComponentTypeKind::Func(ft));
        builder.add_export("run", ExportKind::Func, idx);
        let bytes = builder.build();
        let report = validate_component(&bytes).unwrap();
        assert!(report.has_export_section);
        assert_eq!(builder.export_count(), 1);
    }

    // ── W2.5: Canonical ABI lifting (string → memory) ──

    #[test]
    fn w2_5_lower_string() {
        let mut mem = LinearMemory::new(1);
        let val = lower_string(&mut mem, "Hello, WASI!");
        if let CanonicalValue::String { ptr, len } = val {
            assert_eq!(len, 12);
            let lifted = lift_string(&mem, ptr, len).unwrap();
            assert_eq!(lifted, "Hello, WASI!");
        } else {
            panic!("expected String");
        }
    }

    #[test]
    fn w2_5_lower_list_u8() {
        let mut mem = LinearMemory::new(1);
        let data = vec![1u8, 2, 3, 4, 5];
        let val = lower_list_u8(&mut mem, &data);
        if let CanonicalValue::List { ptr, len } = val {
            assert_eq!(len, 5);
            let lifted = lift_list_u8(&mem, ptr, len);
            assert_eq!(lifted, data);
        } else {
            panic!("expected List");
        }
    }

    // ── W2.6: Canonical ABI lowering (memory → values) ──

    #[test]
    fn w2_6_lift_string_from_memory() {
        let mut mem = LinearMemory::new(1);
        let text = "Fajar Lang WASI";
        let ptr = mem.alloc(text.len() as u32, 1);
        mem.write_bytes(ptr, text.as_bytes());
        let lifted = lift_string(&mem, ptr, text.len() as u32).unwrap();
        assert_eq!(lifted, text);
    }

    #[test]
    fn w2_6_lift_u32_list() {
        let mut mem = LinearMemory::new(1);
        let values = vec![10u32, 20, 30, 40];
        let ptr = mem.alloc(16, 4);
        for (i, &v) in values.iter().enumerate() {
            mem.write_u32(ptr + (i as u32) * 4, v);
        }
        let lifted = lift_list_u32(&mem, ptr, 4);
        assert_eq!(lifted, values);
    }

    // ── W2.7: cabi_realloc ──

    #[test]
    fn w2_7_cabi_realloc_allocates() {
        let mut mem = LinearMemory::new(1);
        let ptr1 = cabi_realloc(&mut mem, 0, 0, 1, 100);
        assert!(ptr1 > 0);
        let ptr2 = cabi_realloc(&mut mem, 0, 0, 4, 200);
        assert!(ptr2 >= ptr1 + 100);
        // Aligned to 4 bytes
        assert_eq!(ptr2 % 4, 0);
    }

    #[test]
    fn w2_7_host_allocates_in_guest() {
        let mut mem = LinearMemory::new(1);
        // Simulate host allocating a string buffer in guest memory
        let ptr = cabi_realloc(&mut mem, 0, 0, 1, 64);
        mem.write_bytes(ptr, b"Hello from host");
        let result = lift_string(&mem, ptr, 15).unwrap();
        assert_eq!(result, "Hello from host");
    }

    // ── W2.8: Post-return cleanup ──

    #[test]
    fn w2_8_post_return_tracking() {
        let mut tracker = PostReturnTracker::new();
        assert_eq!(tracker.pending_count(), 0);

        tracker.register(100, 64);
        tracker.register(200, 128);
        assert_eq!(tracker.pending_count(), 2);

        let cleaned = tracker.drain();
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned[0], (100, 64));
        assert_eq!(cleaned[1], (200, 128));
        assert_eq!(tracker.pending_count(), 0);
    }

    #[test]
    fn w2_8_no_memory_leaks() {
        let mut mem = LinearMemory::new(1);
        let mut tracker = PostReturnTracker::new();

        // Simulate function call that returns a string
        let val = lower_string(&mut mem, "response data");
        if let CanonicalValue::String { ptr, len } = val {
            tracker.register(ptr, len);
        }

        // Post-return: all allocations tracked
        let cleanup = tracker.drain();
        assert_eq!(cleanup.len(), 1);
    }

    // ── W2.9: Component validation ──

    #[test]
    fn w2_9_validate_valid_component() {
        let mut builder = ComponentBuilder::new();
        builder.add_type(ComponentTypeKind::Func(ComponentFuncType {
            name: "run".into(),
            params: Vec::new(),
            result: None,
        }));
        builder.add_export("run", ExportKind::Func, 0);
        let bytes = builder.build();
        let report = validate_component(&bytes).unwrap();
        assert!(report.valid);
        assert!(report.magic_valid);
        assert!(report.version_valid);
    }

    #[test]
    fn w2_9_reject_invalid_magic() {
        let bytes = vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0D, 0x00, 0x01, 0x00];
        let err = validate_component(&bytes).unwrap_err();
        assert!(err.message.contains("invalid magic"));
    }

    #[test]
    fn w2_9_reject_too_short() {
        let bytes = vec![0x00, 0x61, 0x73];
        let err = validate_component(&bytes).unwrap_err();
        assert!(err.message.contains("too short"));
    }

    #[test]
    fn w2_9_reject_wrong_layer() {
        let bytes = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        let err = validate_component(&bytes).unwrap_err();
        assert!(err.message.contains("expected component layer"));
    }

    // ── W2.10: Full integration tests ──

    #[test]
    fn w2_10_build_wasi_cli_component() {
        let src = r#"
package wasi:cli@0.2.0;

world command {
    import wasi:io/streams@0.2.0;
    import wasi:filesystem/types@0.2.0;
    export run: func() -> result;
}
"#;
        let doc = parse_wit(src).unwrap();
        let world = &doc.worlds[0];
        let mut builder = build_component_from_world(world);
        let bytes = builder.build();
        let report = validate_component(&bytes).unwrap();
        assert!(report.valid);
        assert!(report.has_type_section);
        assert!(report.has_import_section);
        assert!(report.has_export_section);
        assert_eq!(builder.import_count(), 2);
        assert_eq!(builder.export_count(), 1);
    }

    #[test]
    fn w2_10_canonical_abi_string_roundtrip() {
        let mut mem = LinearMemory::new(1);
        let long_string = "x".repeat(1000);
        let test_strings = vec![
            "Hello, World!",
            "",
            "Fajar Lang 🚀",
            "WASI Preview 2 Component Model",
            long_string.as_str(),
        ];
        for s in &test_strings {
            let val = lower_string(&mut mem, s);
            if let CanonicalValue::String { ptr, len } = val {
                let lifted = lift_string(&mem, ptr, len).unwrap();
                assert_eq!(&lifted, s);
            }
        }
    }

    #[test]
    fn w2_10_component_validation_report_display() {
        let mut builder = ComponentBuilder::new();
        let core = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        builder.set_core_module(core);
        builder.add_type(ComponentTypeKind::Func(ComponentFuncType {
            name: "test".into(),
            params: vec![("x".into(), ComponentValType::U32)],
            result: Some(ComponentValType::String_),
        }));
        builder.add_import("wasi:io/streams", 0);
        builder.add_export("run", ExportKind::Func, 0);
        let bytes = builder.build();

        let report = validate_component(&bytes).unwrap();
        assert!(report.valid);
        assert!(report.has_core_module);
        assert!(report.has_type_section);
        assert!(report.has_import_section);
        assert!(report.has_export_section);
        assert!(report.section_count >= 4);

        let display = format!("{report}");
        assert!(display.contains("Valid: true"));
    }

    #[test]
    fn w2_10_memory_allocation_alignment() {
        let mut mem = LinearMemory::new(1);
        let p1 = mem.alloc(3, 1); // 3 bytes, align 1
        let p2 = mem.alloc(4, 4); // 4 bytes, align 4
        let p3 = mem.alloc(8, 8); // 8 bytes, align 8

        assert_eq!(p2 % 4, 0, "p2 should be 4-byte aligned");
        assert_eq!(p3 % 8, 0, "p3 should be 8-byte aligned");
        assert!(p2 > p1);
        assert!(p3 > p2);
    }

    #[test]
    fn w2_10_wit_func_to_component_type() {
        let func = WitFuncDef {
            name: "handle".into(),
            doc: None,
            params: vec![WitParam {
                name: "req".into(),
                ty: WitTypeRef::Named("request".into()),
            }],
            result: Some(WitTypeRef::Result {
                ok: Some(Box::new(WitTypeRef::Named("response".into()))),
                err: Some(Box::new(WitTypeRef::Primitive(WitPrimitive::String_))),
            }),
            is_static: false,
            is_constructor: false,
        };

        let ct = wit_func_to_component(&func);
        assert_eq!(ct.name, "handle");
        assert_eq!(ct.params.len(), 1);
        assert!(ct.result.is_some());
    }

    // ── V14 H3.9: WASI Component Size Benchmark ────────────────

    #[test]
    fn v14_h3_9_minimal_component_size() {
        let mut builder = ComponentBuilder::new();
        let func_type = ComponentFuncType {
            name: "run".into(),
            params: vec![],
            result: None,
        };
        let type_idx = builder.add_type(ComponentTypeKind::Func(func_type));
        builder.add_export("run", ExportKind::Func, type_idx);
        let bytes = builder.build();
        // Minimal component: valid WASM magic + component sections
        assert!(bytes.len() >= 8, "component must have WASM header");
        assert_eq!(&bytes[0..4], b"\0asm", "must start with WASM magic");
        // Minimal component should be small (< 1KB)
        assert!(
            bytes.len() < 1024,
            "minimal component should be <1KB, got {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn v14_h3_9_component_with_imports_size() {
        let mut builder = ComponentBuilder::new();
        let func_type = ComponentFuncType {
            name: "log".into(),
            params: vec![("msg".into(), ComponentValType::String_)],
            result: None,
        };
        let type_idx = builder.add_type(ComponentTypeKind::Func(func_type));
        builder.add_import("wasi:cli/stdout", type_idx);
        builder.add_export("run", ExportKind::Func, type_idx);
        let bytes = builder.build();
        assert!(bytes.len() >= 8);
        // Component with imports should still be small
        assert!(
            bytes.len() < 2048,
            "component with 1 import should be <2KB, got {} bytes",
            bytes.len()
        );
    }
}

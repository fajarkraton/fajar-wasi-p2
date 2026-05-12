//! W9: Component Composition & Linking — Sprint W9.
//!
//! Implements component instantiation, cross-component linking, virtualized
//! filesystem overrides, build target configuration, P1→P2 adaptation,
//! multi-component workspaces, import satisfaction checking, WIT dependency
//! resolution, and binary size reporting.
//!
//! ## Tasks
//! - W9.1: `ComponentInstance` — instantiated component with provided imports, run entry
//! - W9.2: `ComponentLinker` — link A's export to B's import by name matching
//! - W9.3: `VirtualFs` — override `wasi:filesystem` with custom in-memory impl
//! - W9.4: `WasiTarget` — `fj build --target wasm32-wasi-p2` CLI config
//! - W9.5: `ComponentAdapter` — wrap P1 module inside P2 component (adapter pattern)
//! - W9.6: `WorkspaceConfig` — multi-component workspace with multiple targets
//! - W9.7: `ImportChecker` — verify all imports satisfied before instantiation
//! - W9.8: `WitRegistry` — resolve `use wasi:*` from standard package registry
//! - W9.9: `SizeReport` — strip debug, optimize sections, report sizes
//! - W9.10: Comprehensive tests (15+)

use super::component::{ComponentBuilder, ComponentFuncType, ComponentTypeKind, ExportKind};
use super::filesystem::WasiFilesystem;
use std::collections::HashMap;
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// W9.1: Component Instantiation
// ═══════════════════════════════════════════════════════════════════════

/// An instantiated component with resolved imports and callable exports.
///
/// `ComponentInstance` holds the component binary, a map of import providers,
/// and the set of exports that can be invoked. The `run()` method executes
/// the component's default export (or `_start`).
#[derive(Debug)]
pub struct ComponentInstance {
    /// Human-readable name for diagnostics.
    name: String,
    /// The raw component binary bytes.
    binary: Vec<u8>,
    /// Import providers: import name -> provider instance name.
    imports: HashMap<String, String>,
    /// Exports offered by this instance: export name -> export definition.
    exports: HashMap<String, ExportDef>,
    /// Whether `run()` has been called (simulated single-shot execution).
    executed: bool,
    /// Simulated return value from `run()`.
    return_value: Option<i32>,
}

/// Definition of an export provided by a component instance.
#[derive(Debug, Clone)]
pub struct ExportDef {
    /// Export name.
    pub name: String,
    /// The kind of export (func, instance, etc.).
    pub kind: ExportKind,
    /// Simulated function signature (params as type names).
    pub params: Vec<String>,
    /// Simulated result type name, if any.
    pub result: Option<String>,
}

/// Errors during component instantiation or execution.
#[derive(Debug, Clone, PartialEq)]
pub enum CompositionError {
    /// A required import is not satisfied.
    MissingImport {
        /// The component needing the import.
        component: String,
        /// The import name that is missing.
        import_name: String,
    },
    /// An export was not found on the source component.
    MissingExport {
        /// The component searched.
        component: String,
        /// The export name requested.
        export_name: String,
    },
    /// Type mismatch when linking export to import.
    TypeMismatch {
        /// Description of the mismatch.
        detail: String,
    },
    /// The component has already been executed.
    AlreadyExecuted {
        /// Component name.
        component: String,
    },
    /// A cycle was detected in the component dependency graph.
    CycleDetected {
        /// Components forming the cycle.
        components: Vec<String>,
    },
    /// Workspace configuration error.
    WorkspaceError {
        /// Error detail.
        detail: String,
    },
    /// WIT registry resolution failed.
    RegistryError {
        /// Error detail.
        detail: String,
    },
    /// Binary optimization error.
    OptimizationError {
        /// Error detail.
        detail: String,
    },
}

impl fmt::Display for CompositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingImport {
                component,
                import_name,
            } => write!(
                f,
                "component '{component}' has unsatisfied import '{import_name}'"
            ),
            Self::MissingExport {
                component,
                export_name,
            } => write!(f, "component '{component}' does not export '{export_name}'"),
            Self::TypeMismatch { detail } => write!(f, "type mismatch: {detail}"),
            Self::AlreadyExecuted { component } => {
                write!(f, "component '{component}' already executed")
            }
            Self::CycleDetected { components } => {
                write!(f, "dependency cycle detected: {}", components.join(" -> "))
            }
            Self::WorkspaceError { detail } => write!(f, "workspace error: {detail}"),
            Self::RegistryError { detail } => write!(f, "registry error: {detail}"),
            Self::OptimizationError { detail } => write!(f, "optimization error: {detail}"),
        }
    }
}

impl ComponentInstance {
    /// Creates a new component instance from a name, binary, and provided imports.
    ///
    /// The `provided_imports` map wires import names to the names of other
    /// component instances that supply those imports.
    pub fn new(
        name: impl Into<String>,
        binary: Vec<u8>,
        provided_imports: HashMap<String, String>,
    ) -> Self {
        Self {
            name: name.into(),
            binary,
            imports: provided_imports,
            exports: HashMap::new(),
            executed: false,
            return_value: None,
        }
    }

    /// Registers an export on this instance.
    pub fn add_export(&mut self, export: ExportDef) {
        self.exports.insert(export.name.clone(), export);
    }

    /// Runs the component's entry point (simulated).
    ///
    /// Returns the simulated exit code. Returns an error if already executed.
    pub fn run(&mut self) -> Result<i32, CompositionError> {
        if self.executed {
            return Err(CompositionError::AlreadyExecuted {
                component: self.name.clone(),
            });
        }
        self.executed = true;
        let code = self.return_value.unwrap_or(0);
        Ok(code)
    }

    /// Sets the simulated return value for `run()`.
    pub fn set_return_value(&mut self, code: i32) {
        self.return_value = Some(code);
    }

    /// Returns the instance name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the raw component binary.
    pub fn binary(&self) -> &[u8] {
        &self.binary
    }

    /// Returns the provided imports map.
    pub fn imports(&self) -> &HashMap<String, String> {
        &self.imports
    }

    /// Returns the registered exports.
    pub fn exports(&self) -> &HashMap<String, ExportDef> {
        &self.exports
    }

    /// Whether this instance has been executed.
    pub fn has_executed(&self) -> bool {
        self.executed
    }

    /// Looks up an export by name.
    pub fn get_export(&self, name: &str) -> Option<&ExportDef> {
        self.exports.get(name)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.2: Component Linker
// ═══════════════════════════════════════════════════════════════════════

/// Links multiple component instances together by wiring exports to imports.
///
/// The linker maintains a registry of named instances. When `link()` is called,
/// it verifies that every import of every instance is satisfied by an export
/// from another registered instance (matched by name).
#[derive(Debug, Default)]
pub struct ComponentLinker {
    /// Registered component instances by name.
    instances: HashMap<String, ComponentInstance>,
    /// Established links: (consumer_name, import_name) -> (provider_name, export_name).
    links: Vec<Link>,
}

/// A single link wiring a provider's export to a consumer's import.
#[derive(Debug, Clone)]
pub struct Link {
    /// The component providing the export.
    pub provider: String,
    /// The export name on the provider.
    pub export_name: String,
    /// The component consuming the import.
    pub consumer: String,
    /// The import name on the consumer.
    pub import_name: String,
}

impl ComponentLinker {
    /// Creates a new empty linker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a component instance with the linker.
    pub fn register(&mut self, instance: ComponentInstance) {
        self.instances.insert(instance.name().to_string(), instance);
    }

    /// Links a provider's export to a consumer's import by name matching.
    ///
    /// Verifies that the provider has the named export and that the consumer
    /// has the named import, then records the link.
    pub fn link(
        &mut self,
        provider_name: &str,
        export_name: &str,
        consumer_name: &str,
        import_name: &str,
    ) -> Result<(), CompositionError> {
        // Verify provider exists and has the export
        let provider =
            self.instances
                .get(provider_name)
                .ok_or_else(|| CompositionError::MissingExport {
                    component: provider_name.to_string(),
                    export_name: export_name.to_string(),
                })?;
        if !provider.exports.contains_key(export_name) {
            return Err(CompositionError::MissingExport {
                component: provider_name.to_string(),
                export_name: export_name.to_string(),
            });
        }

        // Verify consumer exists and has the import
        let consumer =
            self.instances
                .get(consumer_name)
                .ok_or_else(|| CompositionError::MissingImport {
                    component: consumer_name.to_string(),
                    import_name: import_name.to_string(),
                })?;
        if !consumer.imports.contains_key(import_name) {
            return Err(CompositionError::MissingImport {
                component: consumer_name.to_string(),
                import_name: import_name.to_string(),
            });
        }

        self.links.push(Link {
            provider: provider_name.to_string(),
            export_name: export_name.to_string(),
            consumer: consumer_name.to_string(),
            import_name: import_name.to_string(),
        });

        Ok(())
    }

    /// Returns all established links.
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Returns the number of registered instances.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Retrieves a registered instance by name.
    pub fn get_instance(&self, name: &str) -> Option<&ComponentInstance> {
        self.instances.get(name)
    }

    /// Retrieves a mutable reference to a registered instance by name.
    pub fn get_instance_mut(&mut self, name: &str) -> Option<&mut ComponentInstance> {
        self.instances.get_mut(name)
    }

    /// Checks whether all imports across all instances are satisfied by links.
    /// Returns a list of unsatisfied (component, import_name) pairs.
    pub fn check_all_imports(&self) -> Vec<(String, String)> {
        let mut unsatisfied = Vec::new();
        for (name, instance) in &self.instances {
            for import_name in instance.imports.keys() {
                let is_linked = self
                    .links
                    .iter()
                    .any(|l| l.consumer == *name && l.import_name == *import_name);
                if !is_linked {
                    unsatisfied.push((name.clone(), import_name.clone()));
                }
            }
        }
        unsatisfied
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.3: Virtualized Filesystem
// ═══════════════════════════════════════════════════════════════════════

/// Wraps a `WasiFilesystem` to override `wasi:filesystem` with a custom
/// in-memory implementation.
///
/// `VirtualFs` intercepts filesystem operations and routes them through
/// the in-memory `WasiFilesystem`, allowing tests and sandboxed execution
/// without touching the real host filesystem.
#[derive(Debug)]
pub struct VirtualFs {
    /// The underlying in-memory filesystem.
    inner: WasiFilesystem,
    /// Mount points: virtual path prefix -> label.
    mounts: HashMap<String, String>,
    /// Whether the virtual filesystem is read-only.
    read_only: bool,
}

impl VirtualFs {
    /// Creates a new virtual filesystem wrapping a fresh `WasiFilesystem`.
    pub fn new() -> Self {
        Self {
            inner: WasiFilesystem::new(),
            mounts: HashMap::new(),
            read_only: false,
        }
    }

    /// Creates a virtual filesystem from an existing `WasiFilesystem`.
    pub fn from_fs(fs: WasiFilesystem) -> Self {
        Self {
            inner: fs,
            mounts: HashMap::new(),
            read_only: false,
        }
    }

    /// Adds a mount point: a named label mapped to a virtual path prefix.
    pub fn mount(&mut self, path_prefix: impl Into<String>, label: impl Into<String>) {
        self.mounts.insert(path_prefix.into(), label.into());
    }

    /// Sets whether the filesystem is read-only.
    pub fn set_read_only(&mut self, read_only: bool) {
        self.read_only = read_only;
    }

    /// Returns whether the filesystem is read-only.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Returns the list of mount points.
    pub fn mounts(&self) -> &HashMap<String, String> {
        &self.mounts
    }

    /// Returns a mutable reference to the underlying filesystem.
    pub fn inner_mut(&mut self) -> &mut WasiFilesystem {
        &mut self.inner
    }

    /// Returns a reference to the underlying filesystem.
    pub fn inner(&self) -> &WasiFilesystem {
        &self.inner
    }

    /// Pre-populates a file in the virtual filesystem with the given contents.
    ///
    /// Creates intermediate directories as needed.
    pub fn preload_file(&mut self, path: &str, contents: &[u8]) -> Result<(), String> {
        use super::filesystem::{DescriptorFlags, OpenFlags};

        let root = self.inner.open_root();

        // Create parent directories if path has them
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if parts.len() > 1 {
            let dir_path = parts[..parts.len() - 1].join("/");
            self.inner
                .create_directory_at(root, &dir_path)
                .map_err(|e| format!("failed to create directories: {e}"))?;
        }

        let fd = self
            .inner
            .open_at(
                root,
                path,
                OpenFlags {
                    create: true,
                    ..Default::default()
                },
                DescriptorFlags::default(),
            )
            .map_err(|e| format!("failed to open file: {e}"))?;
        self.inner
            .write(fd, contents)
            .map_err(|e| format!("failed to write file: {e}"))?;
        self.inner
            .close(fd)
            .map_err(|e| format!("failed to close file: {e}"))?;
        Ok(())
    }
}

impl Default for VirtualFs {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.4: Build Target Configuration
// ═══════════════════════════════════════════════════════════════════════

/// WASI target for `fj build --target`.
///
/// Distinguishes between WASI Preview 1 (core module) and WASI Preview 2
/// (component model) build outputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasiTarget {
    /// `wasm32-wasi` — produces a core Wasm module with WASI P1 imports.
    WasmWasiP1,
    /// `wasm32-wasi-p2` — produces a component with WASI P2 interfaces.
    WasmWasiP2,
}

impl WasiTarget {
    /// Parses a target triple string into a `WasiTarget`.
    pub fn from_triple(triple: &str) -> Option<Self> {
        match triple {
            "wasm32-wasi" | "wasm32-wasi-p1" => Some(Self::WasmWasiP1),
            "wasm32-wasi-p2" | "wasm32-wasip2" => Some(Self::WasmWasiP2),
            _ => None,
        }
    }

    /// Returns the canonical target triple string.
    pub fn triple(&self) -> &'static str {
        match self {
            Self::WasmWasiP1 => "wasm32-wasi",
            Self::WasmWasiP2 => "wasm32-wasi-p2",
        }
    }

    /// Returns the file extension for the output binary.
    pub fn output_extension(&self) -> &'static str {
        match self {
            Self::WasmWasiP1 => "wasm",
            Self::WasmWasiP2 => "component.wasm",
        }
    }

    /// Whether this target produces a component (vs a core module).
    pub fn is_component(&self) -> bool {
        matches!(self, Self::WasmWasiP2)
    }
}

impl fmt::Display for WasiTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.triple())
    }
}

/// Build configuration for a WASI target.
#[derive(Debug, Clone)]
pub struct WasiBuildConfig {
    /// The target to build for.
    pub target: WasiTarget,
    /// Output file path.
    pub output_path: String,
    /// Whether to strip debug info.
    pub strip_debug: bool,
    /// Whether to optimize for size.
    pub optimize_size: bool,
    /// Additional WIT files to include.
    pub wit_files: Vec<String>,
    /// Feature flags enabled for this build.
    pub features: Vec<String>,
}

impl WasiBuildConfig {
    /// Creates a new build config for the given target.
    pub fn new(target: WasiTarget, output_path: impl Into<String>) -> Self {
        Self {
            target,
            output_path: output_path.into(),
            strip_debug: false,
            optimize_size: false,
            wit_files: Vec::new(),
            features: Vec::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.5: Component Adapter (P1 → P2)
// ═══════════════════════════════════════════════════════════════════════

/// Wraps a WASI P1 core module inside a P2 component.
///
/// The adapter intercepts P1 import calls (`fd_read`, `fd_write`, etc.)
/// and translates them into P2 interface calls (`wasi:io/streams`,
/// `wasi:filesystem/types`, etc.).
#[derive(Debug)]
pub struct ComponentAdapter {
    /// The P1 core module bytes.
    p1_module: Vec<u8>,
    /// Adapter import mappings: P1 import name -> P2 interface path.
    import_map: HashMap<String, String>,
    /// Adapter export mappings: P1 export name -> P2 export name.
    export_map: HashMap<String, String>,
}

impl ComponentAdapter {
    /// Creates a new adapter for the given P1 module bytes.
    pub fn new(p1_module: Vec<u8>) -> Self {
        let mut import_map = HashMap::new();
        // Standard P1→P2 import mappings
        import_map.insert("fd_read".into(), "wasi:io/streams.read".into());
        import_map.insert("fd_write".into(), "wasi:io/streams.write".into());
        import_map.insert("fd_close".into(), "wasi:io/streams.drop".into());
        import_map.insert("path_open".into(), "wasi:filesystem/types.open-at".into());
        import_map.insert(
            "fd_prestat_get".into(),
            "wasi:filesystem/preopens.get-directories".into(),
        );
        import_map.insert(
            "clock_time_get".into(),
            "wasi:clocks/monotonic-clock.now".into(),
        );
        import_map.insert(
            "environ_get".into(),
            "wasi:cli/environment.get-environment".into(),
        );
        import_map.insert(
            "args_get".into(),
            "wasi:cli/environment.get-arguments".into(),
        );
        import_map.insert("proc_exit".into(), "wasi:cli/exit.exit".into());
        import_map.insert(
            "random_get".into(),
            "wasi:random/random.get-random-bytes".into(),
        );

        let mut export_map = HashMap::new();
        export_map.insert("_start".into(), "wasi:cli/run.run".into());
        export_map.insert("memory".into(), "memory".into());

        Self {
            p1_module,
            import_map,
            export_map,
        }
    }

    /// Adapts the P1 module into a P2 component binary.
    ///
    /// Builds a `ComponentBuilder` that wraps the core module with the
    /// appropriate P2 import/export sections.
    pub fn adapt(&self) -> Vec<u8> {
        let mut builder = ComponentBuilder::new();
        builder.set_core_module(self.p1_module.clone());

        // Add P2 imports for each mapped P1 import
        for p2_path in self.import_map.values() {
            let idx = builder.add_type(ComponentTypeKind::Instance(Vec::new()));
            builder.add_import(p2_path, idx);
        }

        // Add P2 exports for each mapped P1 export
        for p2_name in self.export_map.values() {
            let idx = builder.add_type(ComponentTypeKind::Func(ComponentFuncType {
                name: p2_name.clone(),
                params: Vec::new(),
                result: None,
            }));
            builder.add_export(p2_name, ExportKind::Func, idx);
        }

        builder.enable_realloc();
        builder.build()
    }

    /// Returns the P1→P2 import mapping.
    pub fn import_map(&self) -> &HashMap<String, String> {
        &self.import_map
    }

    /// Returns the P1→P2 export mapping.
    pub fn export_map(&self) -> &HashMap<String, String> {
        &self.export_map
    }

    /// Adds a custom import mapping.
    pub fn add_import_mapping(&mut self, p1_name: impl Into<String>, p2_path: impl Into<String>) {
        self.import_map.insert(p1_name.into(), p2_path.into());
    }

    /// Adds a custom export mapping.
    pub fn add_export_mapping(&mut self, p1_name: impl Into<String>, p2_name: impl Into<String>) {
        self.export_map.insert(p1_name.into(), p2_name.into());
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.6: Multi-Component Workspace
// ═══════════════════════════════════════════════════════════════════════

/// Workspace configuration for multi-component packages.
///
/// A workspace defines multiple component targets that can reference each
/// other's exports. Each member has its own source, target, and dependencies.
#[derive(Debug, Clone)]
pub struct WorkspaceConfig {
    /// Workspace name.
    pub name: String,
    /// Workspace members (component targets).
    pub members: Vec<WorkspaceMember>,
    /// Shared feature flags across all members.
    pub shared_features: Vec<String>,
}

/// A single component target within a workspace.
#[derive(Debug, Clone)]
pub struct WorkspaceMember {
    /// Member name (used as component name).
    pub name: String,
    /// Source directory relative to workspace root.
    pub source_dir: String,
    /// Build target for this member.
    pub target: WasiTarget,
    /// Dependencies on other workspace members (by name).
    pub depends_on: Vec<String>,
    /// Per-member feature flags.
    pub features: Vec<String>,
}

impl WorkspaceConfig {
    /// Creates a new workspace with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            members: Vec::new(),
            shared_features: Vec::new(),
        }
    }

    /// Adds a member to the workspace.
    pub fn add_member(&mut self, member: WorkspaceMember) {
        self.members.push(member);
    }

    /// Validates the workspace configuration.
    ///
    /// Checks:
    /// - No duplicate member names.
    /// - All `depends_on` references point to existing members.
    /// - No circular dependencies.
    pub fn validate(&self) -> Result<(), CompositionError> {
        let names: Vec<&str> = self.members.iter().map(|m| m.name.as_str()).collect();

        // Check for duplicates
        let mut seen = std::collections::HashSet::new();
        for name in &names {
            if !seen.insert(name) {
                return Err(CompositionError::WorkspaceError {
                    detail: format!("duplicate member name: '{name}'"),
                });
            }
        }

        // Check that all depends_on targets exist
        for member in &self.members {
            for dep in &member.depends_on {
                if !names.contains(&dep.as_str()) {
                    return Err(CompositionError::WorkspaceError {
                        detail: format!(
                            "member '{}' depends on '{}' which does not exist in workspace",
                            member.name, dep
                        ),
                    });
                }
            }
        }

        // Check for cycles using DFS
        let mut visited = std::collections::HashSet::new();
        let mut stack = std::collections::HashSet::new();
        for member in &self.members {
            if !visited.contains(&member.name) {
                self.detect_cycle(&member.name, &mut visited, &mut stack)?;
            }
        }

        Ok(())
    }

    /// DFS cycle detection helper.
    fn detect_cycle(
        &self,
        name: &str,
        visited: &mut std::collections::HashSet<String>,
        stack: &mut std::collections::HashSet<String>,
    ) -> Result<(), CompositionError> {
        visited.insert(name.to_string());
        stack.insert(name.to_string());

        if let Some(member) = self.members.iter().find(|m| m.name == name) {
            for dep in &member.depends_on {
                if !visited.contains(dep.as_str()) {
                    self.detect_cycle(dep, visited, stack)?;
                } else if stack.contains(dep.as_str()) {
                    return Err(CompositionError::CycleDetected {
                        components: vec![name.to_string(), dep.clone()],
                    });
                }
            }
        }

        stack.remove(name);
        Ok(())
    }

    /// Returns a topological build order for the members.
    ///
    /// Members with no dependencies come first, then those that depend on them.
    pub fn build_order(&self) -> Result<Vec<String>, CompositionError> {
        self.validate()?;

        let mut order = Vec::new();
        let mut built: std::collections::HashSet<String> = std::collections::HashSet::new();
        let member_count = self.members.len();

        // Simple iterative topological sort
        for _ in 0..member_count {
            for member in &self.members {
                if built.contains(&member.name) {
                    continue;
                }
                let deps_met = member.depends_on.iter().all(|d| built.contains(d));
                if deps_met {
                    order.push(member.name.clone());
                    built.insert(member.name.clone());
                }
            }
        }

        if order.len() != member_count {
            return Err(CompositionError::CycleDetected {
                components: self
                    .members
                    .iter()
                    .filter(|m| !built.contains(&m.name))
                    .map(|m| m.name.clone())
                    .collect(),
            });
        }

        Ok(order)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.7: Import Satisfaction Check
// ═══════════════════════════════════════════════════════════════════════

/// Verifies that all imports of a component are satisfied before instantiation.
///
/// Produces clear diagnostics for each missing import, including the interface
/// path and expected type.
#[derive(Debug, Default)]
pub struct ImportChecker {
    /// Required imports: name -> expected interface description.
    required: HashMap<String, ImportRequirement>,
    /// Provided imports: name -> provider description.
    provided: HashMap<String, String>,
}

/// A single required import with metadata.
#[derive(Debug, Clone)]
pub struct ImportRequirement {
    /// The interface name (e.g., `wasi:io/streams@0.2.0`).
    pub interface_name: String,
    /// Human-readable description of what the import provides.
    pub description: String,
    /// Whether this import is optional (soft requirement).
    pub optional: bool,
}

/// Result of an import satisfaction check.
#[derive(Debug, Clone)]
pub struct ImportCheckResult {
    /// Whether all required (non-optional) imports are satisfied.
    pub satisfied: bool,
    /// List of missing required imports.
    pub missing: Vec<ImportRequirement>,
    /// List of missing optional imports (warnings, not errors).
    pub missing_optional: Vec<ImportRequirement>,
    /// List of satisfied imports.
    pub satisfied_imports: Vec<String>,
}

impl ImportChecker {
    /// Creates a new import checker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a required import.
    pub fn require(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        optional: bool,
    ) {
        let name = name.into();
        self.required.insert(
            name.clone(),
            ImportRequirement {
                interface_name: name,
                description: description.into(),
                optional,
            },
        );
    }

    /// Marks an import as provided by the given provider.
    pub fn provide(&mut self, name: impl Into<String>, provider: impl Into<String>) {
        self.provided.insert(name.into(), provider.into());
    }

    /// Checks whether all imports are satisfied.
    ///
    /// Returns a detailed report including missing required, missing optional,
    /// and satisfied imports.
    pub fn check(&self) -> ImportCheckResult {
        let mut missing = Vec::new();
        let mut missing_optional = Vec::new();
        let mut satisfied_imports = Vec::new();

        for (name, req) in &self.required {
            if self.provided.contains_key(name) {
                satisfied_imports.push(name.clone());
            } else if req.optional {
                missing_optional.push(req.clone());
            } else {
                missing.push(req.clone());
            }
        }

        ImportCheckResult {
            satisfied: missing.is_empty(),
            missing,
            missing_optional,
            satisfied_imports,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.8: WIT Dependency Resolution
// ═══════════════════════════════════════════════════════════════════════

/// Resolves `use wasi:*` references against a registry of standard WASI
/// interface definitions.
///
/// The registry maps interface paths (e.g., `wasi:io/streams@0.2.0`) to
/// their WIT descriptions (list of function names and types).
#[derive(Debug, Default)]
pub struct WitRegistry {
    /// Registered interface packages: full path -> list of function names.
    interfaces: HashMap<String, WitInterfaceEntry>,
}

/// A registered WIT interface entry.
#[derive(Debug, Clone)]
pub struct WitInterfaceEntry {
    /// Full interface path (e.g., `wasi:io/streams@0.2.0`).
    pub path: String,
    /// Function names provided by this interface.
    pub functions: Vec<String>,
    /// Version string.
    pub version: String,
    /// Short description.
    pub description: String,
}

impl WitRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a registry pre-populated with standard WASI P2 interfaces.
    pub fn with_standard_wasi() -> Self {
        let mut reg = Self::new();

        reg.register(WitInterfaceEntry {
            path: "wasi:io/streams@0.2.0".into(),
            functions: vec![
                "read".into(),
                "blocking-read".into(),
                "write".into(),
                "blocking-write-and-flush".into(),
                "subscribe".into(),
                "drop".into(),
            ],
            version: "0.2.0".into(),
            description: "WASI I/O streams interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:filesystem/types@0.2.0".into(),
            functions: vec![
                "open-at".into(),
                "read-via-stream".into(),
                "write-via-stream".into(),
                "stat".into(),
                "stat-at".into(),
                "readdir".into(),
                "create-directory-at".into(),
                "unlink-file-at".into(),
                "rename-at".into(),
            ],
            version: "0.2.0".into(),
            description: "WASI filesystem types interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:filesystem/preopens@0.2.0".into(),
            functions: vec!["get-directories".into()],
            version: "0.2.0".into(),
            description: "WASI filesystem preopens interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:cli/environment@0.2.0".into(),
            functions: vec![
                "get-environment".into(),
                "get-arguments".into(),
                "initial-cwd".into(),
            ],
            version: "0.2.0".into(),
            description: "WASI CLI environment interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:cli/exit@0.2.0".into(),
            functions: vec!["exit".into()],
            version: "0.2.0".into(),
            description: "WASI CLI exit interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:clocks/monotonic-clock@0.2.0".into(),
            functions: vec![
                "now".into(),
                "resolution".into(),
                "subscribe-instant".into(),
            ],
            version: "0.2.0".into(),
            description: "WASI monotonic clock interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:clocks/wall-clock@0.2.0".into(),
            functions: vec!["now".into(), "resolution".into()],
            version: "0.2.0".into(),
            description: "WASI wall clock interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:random/random@0.2.0".into(),
            functions: vec!["get-random-bytes".into(), "get-random-u64".into()],
            version: "0.2.0".into(),
            description: "WASI random number interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:http/types@0.2.0".into(),
            functions: vec![
                "new-outgoing-request".into(),
                "outgoing-request-write".into(),
                "incoming-response-consume".into(),
            ],
            version: "0.2.0".into(),
            description: "WASI HTTP types interface".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:http/outgoing-handler@0.2.0".into(),
            functions: vec!["handle".into()],
            version: "0.2.0".into(),
            description: "WASI HTTP outgoing request handler".into(),
        });

        reg.register(WitInterfaceEntry {
            path: "wasi:sockets/tcp@0.2.0".into(),
            functions: vec![
                "create-tcp-socket".into(),
                "bind".into(),
                "connect".into(),
                "listen".into(),
                "accept".into(),
            ],
            version: "0.2.0".into(),
            description: "WASI TCP socket interface".into(),
        });

        reg
    }

    /// Registers an interface entry.
    pub fn register(&mut self, entry: WitInterfaceEntry) {
        self.interfaces.insert(entry.path.clone(), entry);
    }

    /// Resolves an interface path. Returns the entry if found.
    pub fn resolve(&self, path: &str) -> Result<&WitInterfaceEntry, CompositionError> {
        self.interfaces
            .get(path)
            .ok_or_else(|| CompositionError::RegistryError {
                detail: format!("interface '{path}' not found in registry"),
            })
    }

    /// Resolves a path with version flexibility: tries exact match first,
    /// then tries matching without version suffix.
    pub fn resolve_flexible(&self, path: &str) -> Result<&WitInterfaceEntry, CompositionError> {
        // Try exact match
        if let Some(entry) = self.interfaces.get(path) {
            return Ok(entry);
        }

        // Try matching base path (strip @version)
        let base = path.split('@').next().unwrap_or(path);
        for (key, entry) in &self.interfaces {
            let key_base = key.split('@').next().unwrap_or(key);
            if key_base == base {
                return Ok(entry);
            }
        }

        Err(CompositionError::RegistryError {
            detail: format!(
                "interface '{path}' not found in registry (also tried without version)"
            ),
        })
    }

    /// Lists all registered interface paths.
    pub fn list_interfaces(&self) -> Vec<&str> {
        let mut paths: Vec<&str> = self.interfaces.keys().map(|s| s.as_str()).collect();
        paths.sort();
        paths
    }

    /// Returns the total number of registered interfaces.
    pub fn interface_count(&self) -> usize {
        self.interfaces.len()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W9.9: Component Binary Size Reporting
// ═══════════════════════════════════════════════════════════════════════

/// Reports component binary sizes, with optional debug stripping and
/// section-level breakdown.
#[derive(Debug, Clone)]
pub struct SizeReport {
    /// Total binary size in bytes.
    pub total_bytes: usize,
    /// Size after stripping debug sections.
    pub stripped_bytes: usize,
    /// Per-section sizes.
    pub sections: Vec<SectionSize>,
    /// Size of the core module (if present).
    pub core_module_bytes: usize,
    /// Size of type sections.
    pub type_section_bytes: usize,
    /// Size of import sections.
    pub import_section_bytes: usize,
    /// Size of export sections.
    pub export_section_bytes: usize,
}

/// Size of a single section in the component binary.
#[derive(Debug, Clone)]
pub struct SectionSize {
    /// Section identifier.
    pub id: u8,
    /// Human-readable section name.
    pub name: String,
    /// Size in bytes.
    pub bytes: usize,
}

impl SizeReport {
    /// Analyzes a component binary and produces a size report.
    ///
    /// Walks the binary sections, computes per-section sizes, and estimates
    /// the stripped size (total minus any custom/debug sections).
    pub fn analyze(binary: &[u8]) -> Result<Self, CompositionError> {
        if binary.len() < 8 {
            return Err(CompositionError::OptimizationError {
                detail: "binary too short to analyze".into(),
            });
        }

        let mut sections = Vec::new();
        let mut core_module_bytes = 0;
        let mut type_section_bytes = 0;
        let mut import_section_bytes = 0;
        let mut export_section_bytes = 0;
        let mut debug_bytes = 0;

        let mut pos = 8; // skip magic + version
        while pos < binary.len() {
            let section_id = binary[pos];
            pos += 1;

            // Read LEB128 section size
            let (size, consumed) = read_leb128_local(&binary[pos..]);
            pos += consumed;

            let section_name = match section_id {
                0x00 => {
                    core_module_bytes = size as usize;
                    "core:module"
                }
                0x01 => {
                    type_section_bytes = size as usize;
                    "type"
                }
                0x02 => {
                    import_section_bytes = size as usize;
                    "import"
                }
                0x03 => {
                    export_section_bytes = size as usize;
                    "export"
                }
                0x04 => "canon",
                0x05 => "core:instance",
                0x06 => "instance",
                0x07 => "alias",
                0x08 => "core:type",
                0x0A => "start",
                0x0B => {
                    debug_bytes += size as usize;
                    "custom"
                }
                _ => "unknown",
            };

            sections.push(SectionSize {
                id: section_id,
                name: section_name.to_string(),
                bytes: size as usize,
            });

            pos += size as usize;
        }

        let total_bytes = binary.len();
        let stripped_bytes = total_bytes.saturating_sub(debug_bytes);

        Ok(Self {
            total_bytes,
            stripped_bytes,
            sections,
            core_module_bytes,
            type_section_bytes,
            import_section_bytes,
            export_section_bytes,
        })
    }

    /// Returns the percentage of the binary occupied by the core module.
    pub fn core_module_percent(&self) -> f64 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.core_module_bytes as f64 / self.total_bytes as f64) * 100.0
    }

    /// Returns the number of sections.
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Returns the savings from stripping debug info.
    pub fn strip_savings(&self) -> usize {
        self.total_bytes.saturating_sub(self.stripped_bytes)
    }
}

impl fmt::Display for SizeReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Component Size Report")?;
        writeln!(f, "  Total:           {} bytes", self.total_bytes)?;
        writeln!(f, "  Stripped:        {} bytes", self.stripped_bytes)?;
        writeln!(f, "  Debug savings:   {} bytes", self.strip_savings())?;
        writeln!(f, "  Sections:        {}", self.sections.len())?;
        for section in &self.sections {
            writeln!(
                f,
                "    [{:02x}] {:16} {} bytes",
                section.id, section.name, section.bytes
            )?;
        }
        Ok(())
    }
}

/// LEB128 decoder (local to this module to avoid cross-module coupling).
fn read_leb128_local(bytes: &[u8]) -> (u32, usize) {
    let mut result: u32 = 0;
    let mut shift = 0;
    let mut consumed = 0;
    for &byte in bytes {
        result |= ((byte & 0x7F) as u32) << shift;
        consumed += 1;
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
// W9.10: Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::ComponentValType;

    // ── W9.1: Component Instantiation ──

    #[test]
    fn w9_1_component_instance_creation_and_run() {
        let mut instance = ComponentInstance::new(
            "test-component",
            vec![0x00, 0x61, 0x73, 0x6D],
            HashMap::new(),
        );
        assert_eq!(instance.name(), "test-component");
        assert!(!instance.has_executed());

        let code = instance.run().unwrap();
        assert_eq!(code, 0);
        assert!(instance.has_executed());
    }

    #[test]
    fn w9_1_component_instance_double_run_fails() {
        let mut instance = ComponentInstance::new("double", vec![0x00], HashMap::new());
        instance.run().unwrap();
        let err = instance.run().unwrap_err();
        assert_eq!(
            err,
            CompositionError::AlreadyExecuted {
                component: "double".into()
            }
        );
    }

    #[test]
    fn w9_1_component_instance_with_exports() {
        let mut instance = ComponentInstance::new("server", vec![], HashMap::new());
        instance.add_export(ExportDef {
            name: "handle-request".into(),
            kind: ExportKind::Func,
            params: vec!["request".into()],
            result: Some("response".into()),
        });
        assert!(instance.get_export("handle-request").is_some());
        assert!(instance.get_export("nonexistent").is_none());
    }

    #[test]
    fn w9_1_component_instance_with_custom_return_value() {
        let mut instance = ComponentInstance::new("erroring", vec![], HashMap::new());
        instance.set_return_value(42);
        let code = instance.run().unwrap();
        assert_eq!(code, 42);
    }

    // ── W9.2: Component Linker ──

    #[test]
    fn w9_2_linker_registers_and_links_components() {
        let mut linker = ComponentLinker::new();

        // Provider: exports "process"
        let mut provider = ComponentInstance::new("processor", vec![], HashMap::new());
        provider.add_export(ExportDef {
            name: "process".into(),
            kind: ExportKind::Func,
            params: vec!["data".into()],
            result: Some("result".into()),
        });

        // Consumer: imports "process"
        let mut imports = HashMap::new();
        imports.insert("process".into(), "processor".into());
        let consumer = ComponentInstance::new("app", vec![], imports);

        linker.register(provider);
        linker.register(consumer);

        linker
            .link("processor", "process", "app", "process")
            .unwrap();
        assert_eq!(linker.links().len(), 1);
        assert_eq!(linker.instance_count(), 2);
    }

    #[test]
    fn w9_2_linker_missing_export_fails() {
        let mut linker = ComponentLinker::new();
        let provider = ComponentInstance::new("empty", vec![], HashMap::new());
        let mut imports = HashMap::new();
        imports.insert("needed".into(), "empty".into());
        let consumer = ComponentInstance::new("needy", vec![], imports);

        linker.register(provider);
        linker.register(consumer);

        let err = linker
            .link("empty", "not-here", "needy", "needed")
            .unwrap_err();
        match err {
            CompositionError::MissingExport { component, .. } => {
                assert_eq!(component, "empty");
            }
            other => panic!("expected MissingExport, got: {other:?}"),
        }
    }

    #[test]
    fn w9_2_linker_check_all_imports() {
        let mut linker = ComponentLinker::new();

        let mut imports = HashMap::new();
        imports.insert("wasi:io/streams".into(), "host".into());
        imports.insert("wasi:filesystem/types".into(), "host".into());
        let consumer = ComponentInstance::new("app", vec![], imports);
        linker.register(consumer);

        let unsatisfied = linker.check_all_imports();
        assert_eq!(unsatisfied.len(), 2);
    }

    // ── W9.3: Virtual Filesystem ──

    #[test]
    fn w9_3_virtual_fs_preload_and_read() {
        let mut vfs = VirtualFs::new();
        vfs.preload_file("src/main.fj", b"fn main() { println(\"hello\") }")
            .unwrap();

        let root = vfs.inner_mut().open_root();
        let stat = vfs.inner().stat_at(root, "src/main.fj").unwrap();
        assert_eq!(stat.size, 30);
    }

    #[test]
    fn w9_3_virtual_fs_mounts_and_read_only() {
        let mut vfs = VirtualFs::new();
        vfs.mount("/app", "application root");
        vfs.mount("/tmp", "temporary files");
        vfs.set_read_only(true);

        assert!(vfs.is_read_only());
        assert_eq!(vfs.mounts().len(), 2);
    }

    // ── W9.4: Build Target ──

    #[test]
    fn w9_4_wasi_target_parsing_and_properties() {
        assert_eq!(
            WasiTarget::from_triple("wasm32-wasi"),
            Some(WasiTarget::WasmWasiP1)
        );
        assert_eq!(
            WasiTarget::from_triple("wasm32-wasi-p2"),
            Some(WasiTarget::WasmWasiP2)
        );
        assert_eq!(
            WasiTarget::from_triple("wasm32-wasip2"),
            Some(WasiTarget::WasmWasiP2)
        );
        assert_eq!(WasiTarget::from_triple("x86_64-linux"), None);

        let p2 = WasiTarget::WasmWasiP2;
        assert!(p2.is_component());
        assert_eq!(p2.output_extension(), "component.wasm");
        assert_eq!(p2.triple(), "wasm32-wasi-p2");
        assert_eq!(p2.to_string(), "wasm32-wasi-p2");

        let p1 = WasiTarget::WasmWasiP1;
        assert!(!p1.is_component());
        assert_eq!(p1.output_extension(), "wasm");
    }

    // ── W9.5: Component Adapter ──

    #[test]
    fn w9_5_adapter_wraps_p1_module_into_p2_component() {
        // Minimal valid P1 module: magic + version
        let p1_bytes = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        let adapter = ComponentAdapter::new(p1_bytes);

        assert!(adapter.import_map().contains_key("fd_read"));
        assert!(adapter.import_map().contains_key("fd_write"));
        assert!(adapter.import_map().contains_key("proc_exit"));
        assert!(adapter.export_map().contains_key("_start"));

        let component_binary = adapter.adapt();
        // Component magic: \0asm with layer 0x0d
        assert_eq!(&component_binary[0..4], b"\x00asm");
        assert_eq!(component_binary[4], 0x0D);
        assert!(component_binary.len() > 8);
    }

    #[test]
    fn w9_5_adapter_custom_mappings() {
        let mut adapter = ComponentAdapter::new(vec![0x00, 0x61, 0x73, 0x6D]);
        adapter.add_import_mapping("custom_import", "wasi:custom/iface.op");
        adapter.add_export_mapping("custom_export", "wasi:custom/iface.result");

        assert_eq!(
            adapter.import_map().get("custom_import"),
            Some(&"wasi:custom/iface.op".to_string())
        );
        assert_eq!(
            adapter.export_map().get("custom_export"),
            Some(&"wasi:custom/iface.result".to_string())
        );
    }

    // ── W9.6: Workspace ──

    #[test]
    fn w9_6_workspace_validation_and_build_order() {
        let mut ws = WorkspaceConfig::new("my-workspace");
        ws.add_member(WorkspaceMember {
            name: "core-lib".into(),
            source_dir: "core".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec![],
            features: vec![],
        });
        ws.add_member(WorkspaceMember {
            name: "app".into(),
            source_dir: "app".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec!["core-lib".into()],
            features: vec![],
        });
        ws.add_member(WorkspaceMember {
            name: "cli".into(),
            source_dir: "cli".into(),
            target: WasiTarget::WasmWasiP1,
            depends_on: vec!["core-lib".into()],
            features: vec![],
        });

        ws.validate().unwrap();
        let order = ws.build_order().unwrap();
        assert_eq!(order[0], "core-lib");
        assert!(order.contains(&"app".to_string()));
        assert!(order.contains(&"cli".to_string()));
    }

    #[test]
    fn w9_6_workspace_cycle_detection() {
        let mut ws = WorkspaceConfig::new("cyclic");
        ws.add_member(WorkspaceMember {
            name: "a".into(),
            source_dir: "a".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec!["b".into()],
            features: vec![],
        });
        ws.add_member(WorkspaceMember {
            name: "b".into(),
            source_dir: "b".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec!["a".into()],
            features: vec![],
        });

        let err = ws.validate().unwrap_err();
        match err {
            CompositionError::CycleDetected { .. } => {} // expected
            other => panic!("expected CycleDetected, got: {other:?}"),
        }
    }

    #[test]
    fn w9_6_workspace_duplicate_member_fails() {
        let mut ws = WorkspaceConfig::new("dupes");
        ws.add_member(WorkspaceMember {
            name: "a".into(),
            source_dir: "a1".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec![],
            features: vec![],
        });
        ws.add_member(WorkspaceMember {
            name: "a".into(),
            source_dir: "a2".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec![],
            features: vec![],
        });

        let err = ws.validate().unwrap_err();
        match err {
            CompositionError::WorkspaceError { detail } => {
                assert!(detail.contains("duplicate"));
            }
            other => panic!("expected WorkspaceError, got: {other:?}"),
        }
    }

    // ── W9.7: Import Checker ──

    #[test]
    fn w9_7_import_checker_all_satisfied() {
        let mut checker = ImportChecker::new();
        checker.require("wasi:io/streams", "I/O streams", false);
        checker.require("wasi:filesystem/types", "filesystem ops", false);
        checker.require("wasi:logging/log", "logging (optional)", true);

        checker.provide("wasi:io/streams", "host");
        checker.provide("wasi:filesystem/types", "host");

        let result = checker.check();
        assert!(result.satisfied);
        assert_eq!(result.missing.len(), 0);
        assert_eq!(result.missing_optional.len(), 1);
        assert_eq!(result.satisfied_imports.len(), 2);
    }

    #[test]
    fn w9_7_import_checker_missing_required() {
        let mut checker = ImportChecker::new();
        checker.require("wasi:io/streams", "I/O streams", false);
        checker.require("wasi:filesystem/types", "filesystem ops", false);

        checker.provide("wasi:io/streams", "host");
        // wasi:filesystem/types NOT provided

        let result = checker.check();
        assert!(!result.satisfied);
        assert_eq!(result.missing.len(), 1);
        assert_eq!(result.missing[0].interface_name, "wasi:filesystem/types");
    }

    // ── W9.8: WIT Registry ──

    #[test]
    fn w9_8_wit_registry_standard_resolution() {
        let reg = WitRegistry::with_standard_wasi();

        let streams = reg.resolve("wasi:io/streams@0.2.0").unwrap();
        assert!(streams.functions.contains(&"read".to_string()));
        assert!(streams.functions.contains(&"write".to_string()));

        let fs = reg.resolve("wasi:filesystem/types@0.2.0").unwrap();
        assert!(fs.functions.contains(&"open-at".to_string()));

        // Unknown interface fails
        let err = reg.resolve("wasi:unknown/iface@0.2.0").unwrap_err();
        match err {
            CompositionError::RegistryError { .. } => {}
            other => panic!("expected RegistryError, got: {other:?}"),
        }
    }

    #[test]
    fn w9_8_wit_registry_flexible_resolution() {
        let reg = WitRegistry::with_standard_wasi();

        // Without version should still resolve
        let entry = reg.resolve_flexible("wasi:io/streams").unwrap();
        assert_eq!(entry.version, "0.2.0");

        // With matching version works too
        let entry = reg.resolve_flexible("wasi:io/streams@0.2.0").unwrap();
        assert_eq!(entry.path, "wasi:io/streams@0.2.0");
    }

    #[test]
    fn w9_8_wit_registry_lists_all_interfaces() {
        let reg = WitRegistry::with_standard_wasi();
        let interfaces = reg.list_interfaces();
        assert!(interfaces.len() >= 10);
        assert!(interfaces.contains(&"wasi:io/streams@0.2.0"));
        assert!(interfaces.contains(&"wasi:cli/exit@0.2.0"));
        assert!(interfaces.contains(&"wasi:http/types@0.2.0"));
    }

    // ── W9.9: Size Report ──

    #[test]
    fn w9_9_size_report_on_built_component() {
        let mut builder = ComponentBuilder::new();
        builder.set_core_module(vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
        let idx = builder.add_type(ComponentTypeKind::Func(ComponentFuncType {
            name: "run".into(),
            params: vec![],
            result: Some(ComponentValType::U32),
        }));
        builder.add_export("run", ExportKind::Func, idx);
        let binary = builder.build();

        let report = SizeReport::analyze(&binary).unwrap();
        assert!(report.total_bytes > 0);
        assert!(report.section_count() > 0);
        assert!(report.core_module_bytes > 0);

        // Display works
        let display = format!("{report}");
        assert!(display.contains("Component Size Report"));
        assert!(display.contains("Total:"));
    }

    #[test]
    fn w9_9_size_report_too_short_binary() {
        let err = SizeReport::analyze(&[0x00, 0x01]).unwrap_err();
        match err {
            CompositionError::OptimizationError { detail } => {
                assert!(detail.contains("too short"));
            }
            other => panic!("expected OptimizationError, got: {other:?}"),
        }
    }

    // ── W9.10: Integration / Comprehensive ──

    #[test]
    fn w9_10_full_composition_workflow() {
        // 1. Build two components via ComponentBuilder
        let mut lib_builder = ComponentBuilder::new();
        let lib_type = lib_builder.add_type(ComponentTypeKind::Func(ComponentFuncType {
            name: "compute".into(),
            params: vec![("x".into(), ComponentValType::S32)],
            result: Some(ComponentValType::S32),
        }));
        lib_builder.add_export("compute", ExportKind::Func, lib_type);
        let lib_binary = lib_builder.build();

        let mut app_builder = ComponentBuilder::new();
        let app_import_type = app_builder.add_type(ComponentTypeKind::Func(ComponentFuncType {
            name: "compute".into(),
            params: vec![("x".into(), ComponentValType::S32)],
            result: Some(ComponentValType::S32),
        }));
        app_builder.add_import("compute", app_import_type);
        let app_binary = app_builder.build();

        // 2. Create instances
        let mut lib_instance = ComponentInstance::new("lib", lib_binary, HashMap::new());
        lib_instance.add_export(ExportDef {
            name: "compute".into(),
            kind: ExportKind::Func,
            params: vec!["x: s32".into()],
            result: Some("s32".into()),
        });

        let mut app_imports = HashMap::new();
        app_imports.insert("compute".into(), "lib".into());
        let app_instance = ComponentInstance::new("app", app_binary, app_imports);

        // 3. Link them
        let mut linker = ComponentLinker::new();
        linker.register(lib_instance);
        linker.register(app_instance);
        linker.link("lib", "compute", "app", "compute").unwrap();

        // 4. Verify all imports satisfied
        let unsatisfied = linker.check_all_imports();
        assert!(unsatisfied.is_empty());

        // 5. Run the app
        let app = linker.get_instance_mut("app").unwrap();
        let code = app.run().unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn w9_10_adapter_then_link_workflow() {
        // Wrap a P1 module in P2, then analyze its size
        let p1_bytes = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        let adapter = ComponentAdapter::new(p1_bytes);
        let component_binary = adapter.adapt();

        // Analyze size
        let report = SizeReport::analyze(&component_binary).unwrap();
        assert!(report.total_bytes > 8);
        assert!(report.core_module_bytes > 0);
        assert!(report.section_count() >= 3); // core module + types + imports + exports

        // The adapted component should have component magic
        assert_eq!(&component_binary[0..4], b"\x00asm");
        assert_eq!(component_binary[4], 0x0D);
    }

    #[test]
    fn w9_10_workspace_build_with_virtual_fs() {
        // Set up workspace
        let mut ws = WorkspaceConfig::new("embedded-ai");
        ws.add_member(WorkspaceMember {
            name: "inference".into(),
            source_dir: "inference/src".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec![],
            features: vec!["simd".into()],
        });
        ws.add_member(WorkspaceMember {
            name: "gateway".into(),
            source_dir: "gateway/src".into(),
            target: WasiTarget::WasmWasiP2,
            depends_on: vec!["inference".into()],
            features: vec!["http".into()],
        });

        let order = ws.build_order().unwrap();
        assert_eq!(order, vec!["inference", "gateway"]);

        // Set up virtual filesystem with source files
        let mut vfs = VirtualFs::new();
        vfs.mount("/workspace", "workspace root");
        vfs.preload_file(
            "inference/src/main.fj",
            b"@device fn infer(x: Tensor) -> Tensor { x }",
        )
        .unwrap();
        vfs.preload_file(
            "gateway/src/main.fj",
            b"fn handle(req: Request) -> Response { ok(200) }",
        )
        .unwrap();

        let root = vfs.inner_mut().open_root();
        let stat = vfs.inner().stat_at(root, "inference/src/main.fj").unwrap();
        assert!(stat.size > 0);
    }

    #[test]
    fn w9_10_import_checker_with_registry() {
        let reg = WitRegistry::with_standard_wasi();
        let mut checker = ImportChecker::new();

        // Require interfaces that exist in registry
        checker.require("wasi:io/streams@0.2.0", "I/O streams", false);
        checker.require("wasi:cli/exit@0.2.0", "process exit", false);

        // Provide all via registry lookup
        for req_name in ["wasi:io/streams@0.2.0", "wasi:cli/exit@0.2.0"] {
            if reg.resolve(req_name).is_ok() {
                checker.provide(req_name, "wasi-host");
            }
        }

        let result = checker.check();
        assert!(result.satisfied);
        assert_eq!(result.satisfied_imports.len(), 2);
    }

    #[test]
    fn w9_10_composition_error_display() {
        let errors = vec![
            CompositionError::MissingImport {
                component: "app".into(),
                import_name: "wasi:io/streams".into(),
            },
            CompositionError::MissingExport {
                component: "lib".into(),
                export_name: "compute".into(),
            },
            CompositionError::TypeMismatch {
                detail: "expected u32 got s32".into(),
            },
            CompositionError::CycleDetected {
                components: vec!["a".into(), "b".into()],
            },
            CompositionError::WorkspaceError {
                detail: "bad config".into(),
            },
            CompositionError::RegistryError {
                detail: "not found".into(),
            },
            CompositionError::OptimizationError {
                detail: "too short".into(),
            },
        ];
        for err in &errors {
            let s = format!("{err}");
            assert!(!s.is_empty());
        }
    }
}

//! W8: Resource Lifecycle & Ownership — WASI Component Model resource handles.
//!
//! Implements the Component Model resource handle semantics:
//! - `HandleTable<T>` — generational handle→value mapping (W8.1)
//! - Drop protocol with cleanup callbacks and auto-drop (W8.2)
//! - Borrow semantics with `BorrowGuard` preventing premature drop (W8.3)
//! - `OwnHandle<T>` vs `BorrowHandle<T>` type distinction (W8.4)
//! - Resource constructor, instance methods, and static methods (W8.5–W8.7)
//! - Nested resource ownership with LIFO drop ordering (W8.8)
//! - Collection support with batch drop (W8.9)

use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

// ═══════════════════════════════════════════════════════════════════════
// W8.1: Resource Handle Table
// ═══════════════════════════════════════════════════════════════════════

/// Generation counter to detect use-after-free. Each slot increments its
/// generation when freed, so stale handles with old generations are rejected.
type Generation = u32;

/// A slot in the handle table: either occupied (with a value and generation)
/// or free (with the generation it was freed at, plus a next-free pointer).
#[derive(Debug)]
enum Slot<T> {
    /// Occupied slot holding a live resource value.
    Occupied { value: T, generation: Generation },
    /// Free slot available for reuse.
    Free {
        generation: Generation,
        next_free: Option<u32>,
    },
}

/// A generational handle table that maps `u32` indices to `T` values.
///
/// Handles carry a generation counter so that stale handles (pointing to
/// a slot that has been freed and potentially reused) are detected and
/// rejected, preventing use-after-free bugs.
///
/// # Design
///
/// - Slots are stored in a dense `Vec` and recycled through a free list.
/// - Each `alloc` returns a handle encoding `(index, generation)`.
/// - Each `free` bumps the slot generation, invalidating old handles.
/// - Borrow tracking: per-slot ref-count prevents drop while borrows are active.
/// - Drop callbacks: optional per-resource-type cleanup function.
/// - Nested resources: parent→children edges for cascading LIFO drop.
#[derive(Debug)]
pub struct HandleTable<T> {
    /// Dense storage of slots.
    slots: Vec<Slot<T>>,
    /// Head of the free list (index into `slots`), or `None` if full.
    free_head: Option<u32>,
    /// Number of currently occupied slots.
    len: u32,
    /// Active borrow count per slot index.
    borrow_counts: HashMap<u32, u32>,
    /// Drop callback invoked when a resource is freed.
    drop_callback: Option<fn(&T)>,
    /// Parent→children edges: parent index → list of child indices.
    children: HashMap<u32, Vec<u32>>,
    /// Child→parent reverse mapping for bookkeeping.
    parent_of: HashMap<u32, u32>,
}

/// Errors that can occur during handle table operations.
#[derive(Debug, Clone, PartialEq)]
pub enum HandleError {
    /// The handle index is out of range.
    InvalidIndex(u32),
    /// The handle's generation does not match the slot's current generation
    /// (the resource was already freed and the slot may have been reused).
    StaleHandle {
        index: u32,
        expected: Generation,
        actual: Generation,
    },
    /// Attempted to free a handle that still has active borrows.
    ActiveBorrows { index: u32, count: u32 },
    /// The table is at maximum capacity (2^32 slots).
    TableFull,
    /// Attempted to operate on an already-freed slot.
    AlreadyFreed(u32),
    /// A cycle would be created in the parent→child relationship.
    CycleDetected { parent: u32, child: u32 },
}

impl fmt::Display for HandleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIndex(idx) => write!(f, "invalid handle index: {idx}"),
            Self::StaleHandle {
                index,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "stale handle at index {index}: expected generation {expected}, got {actual}"
                )
            }
            Self::ActiveBorrows { index, count } => {
                write!(f, "cannot drop handle {index}: {count} active borrow(s)")
            }
            Self::TableFull => write!(f, "handle table is full"),
            Self::AlreadyFreed(idx) => write!(f, "handle {idx} is already freed"),
            Self::CycleDetected { parent, child } => {
                write!(f, "cycle detected: adding child {child} to parent {parent}")
            }
        }
    }
}

impl<T> HandleTable<T> {
    /// Creates a new empty handle table.
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free_head: None,
            len: 0,
            borrow_counts: HashMap::new(),
            drop_callback: None,
            children: HashMap::new(),
            parent_of: HashMap::new(),
        }
    }

    /// Creates a handle table with a pre-allocated capacity.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            slots: Vec::with_capacity(cap),
            free_head: None,
            len: 0,
            borrow_counts: HashMap::new(),
            drop_callback: None,
            children: HashMap::new(),
            parent_of: HashMap::new(),
        }
    }

    /// Sets the drop callback invoked when a resource is freed.
    pub fn set_drop_callback(&mut self, cb: fn(&T)) {
        self.drop_callback = Some(cb);
    }

    /// Allocates a new slot for `value` and returns its raw handle (index, generation).
    pub fn alloc(&mut self, value: T) -> Result<RawHandle, HandleError> {
        if let Some(free_idx) = self.free_head {
            // Reuse a free slot.
            let slot = &self.slots[free_idx as usize];
            let (slot_gen, next) = match slot {
                Slot::Free {
                    generation,
                    next_free,
                } => (*generation, *next_free),
                Slot::Occupied { .. } => return Err(HandleError::InvalidIndex(free_idx)),
            };
            self.slots[free_idx as usize] = Slot::Occupied {
                value,
                generation: slot_gen,
            };
            self.free_head = next;
            self.len += 1;
            Ok(RawHandle {
                index: free_idx,
                generation: slot_gen,
            })
        } else {
            // Append a new slot.
            let idx = self.slots.len();
            if idx > u32::MAX as usize {
                return Err(HandleError::TableFull);
            }
            let idx = idx as u32;
            self.slots.push(Slot::Occupied {
                value,
                generation: 0,
            });
            self.len += 1;
            Ok(RawHandle {
                index: idx,
                generation: 0,
            })
        }
    }

    /// Frees the resource at `handle`, invoking the drop callback if set.
    ///
    /// Returns the removed value on success. Fails if there are active borrows
    /// or the handle is stale/invalid.
    pub fn free(&mut self, handle: RawHandle) -> Result<T, HandleError> {
        self.validate_handle(handle)?;

        // Check for active borrows.
        let borrow_count = self.borrow_counts.get(&handle.index).copied().unwrap_or(0);
        if borrow_count > 0 {
            return Err(HandleError::ActiveBorrows {
                index: handle.index,
                count: borrow_count,
            });
        }

        // Extract the value, bump generation, and link into free list.
        let old_slot = std::mem::replace(
            &mut self.slots[handle.index as usize],
            Slot::Free {
                generation: handle.generation + 1,
                next_free: self.free_head,
            },
        );

        let value = match old_slot {
            Slot::Occupied { value, .. } => value,
            Slot::Free { .. } => return Err(HandleError::AlreadyFreed(handle.index)),
        };

        // Invoke drop callback.
        if let Some(cb) = self.drop_callback {
            cb(&value);
        }

        self.free_head = Some(handle.index);
        self.len -= 1;

        // Clean up borrow tracking.
        self.borrow_counts.remove(&handle.index);

        // Clean up parent/child edges.
        self.parent_of.remove(&handle.index);
        self.children.remove(&handle.index);

        Ok(value)
    }

    /// Frees a handle and recursively drops all nested child resources in LIFO
    /// (reverse) order. Returns the values of all freed resources (parent last).
    pub fn free_recursive(&mut self, handle: RawHandle) -> Result<Vec<T>, HandleError> {
        self.validate_handle(handle)?;

        // Collect children depth-first (they will be dropped first — LIFO).
        let child_handles = self.collect_children_depth_first(handle.index);

        let mut freed = Vec::new();

        // Drop children in reverse order (deepest first = LIFO).
        for child_raw in child_handles.into_iter().rev() {
            // Detach from parent before freeing.
            self.parent_of.remove(&child_raw.index);
            if let Some(siblings) = self.children.get_mut(&handle.index) {
                siblings.retain(|&c| c != child_raw.index);
            }
            match self.free(child_raw) {
                Ok(val) => freed.push(val),
                Err(HandleError::AlreadyFreed(_)) => { /* already cleaned up */ }
                Err(e) => return Err(e),
            }
        }

        // Finally drop the parent itself.
        let parent_val = self.free(handle)?;
        freed.push(parent_val);

        Ok(freed)
    }

    /// Gets an immutable reference to the value behind `handle`.
    pub fn get(&self, handle: RawHandle) -> Result<&T, HandleError> {
        self.validate_handle(handle)?;
        match &self.slots[handle.index as usize] {
            Slot::Occupied { value, .. } => Ok(value),
            Slot::Free { .. } => Err(HandleError::AlreadyFreed(handle.index)),
        }
    }

    /// Gets a mutable reference to the value behind `handle`.
    pub fn get_mut(&mut self, handle: RawHandle) -> Result<&mut T, HandleError> {
        self.validate_handle(handle)?;
        match &mut self.slots[handle.index as usize] {
            Slot::Occupied { value, .. } => Ok(value),
            Slot::Free { .. } => Err(HandleError::AlreadyFreed(handle.index)),
        }
    }

    /// Acquires a borrow on the handle, incrementing the ref-count.
    /// While borrows are active, the handle cannot be freed.
    pub fn borrow(&mut self, handle: RawHandle) -> Result<(), HandleError> {
        self.validate_handle(handle)?;
        let count = self.borrow_counts.entry(handle.index).or_insert(0);
        *count += 1;
        Ok(())
    }

    /// Releases a borrow on the handle, decrementing the ref-count.
    pub fn release_borrow(&mut self, handle: RawHandle) -> Result<(), HandleError> {
        self.validate_handle(handle)?;
        if let Some(count) = self.borrow_counts.get_mut(&handle.index) {
            if *count > 0 {
                *count -= 1;
                if *count == 0 {
                    self.borrow_counts.remove(&handle.index);
                }
            }
        }
        Ok(())
    }

    /// Returns the number of active borrows on a handle.
    pub fn borrow_count(&self, handle: RawHandle) -> Result<u32, HandleError> {
        self.validate_handle(handle)?;
        Ok(self.borrow_counts.get(&handle.index).copied().unwrap_or(0))
    }

    /// Registers `child` as a nested resource of `parent`. When `parent` is
    /// freed via `free_recursive`, `child` will also be freed (LIFO order).
    pub fn add_child(&mut self, parent: RawHandle, child: RawHandle) -> Result<(), HandleError> {
        self.validate_handle(parent)?;
        self.validate_handle(child)?;

        // Prevent cycles: child must not be an ancestor of parent.
        if self.is_ancestor(child.index, parent.index) {
            return Err(HandleError::CycleDetected {
                parent: parent.index,
                child: child.index,
            });
        }

        self.children
            .entry(parent.index)
            .or_default()
            .push(child.index);
        self.parent_of.insert(child.index, parent.index);
        Ok(())
    }

    /// Returns the number of currently occupied slots.
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Returns `true` if the table has no occupied slots.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the total number of slots (occupied + free).
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Validates that a handle points to a live, matching-generation slot.
    fn validate_handle(&self, handle: RawHandle) -> Result<(), HandleError> {
        if handle.index as usize >= self.slots.len() {
            return Err(HandleError::InvalidIndex(handle.index));
        }
        match &self.slots[handle.index as usize] {
            Slot::Occupied { generation, .. } => {
                if *generation != handle.generation {
                    Err(HandleError::StaleHandle {
                        index: handle.index,
                        expected: *generation,
                        actual: handle.generation,
                    })
                } else {
                    Ok(())
                }
            }
            Slot::Free { generation, .. } => Err(HandleError::StaleHandle {
                index: handle.index,
                expected: *generation,
                actual: handle.generation,
            }),
        }
    }

    /// Checks whether `ancestor_idx` is an ancestor of `descendant_idx` in
    /// the parent→child tree. Used to prevent cycles.
    fn is_ancestor(&self, ancestor_idx: u32, descendant_idx: u32) -> bool {
        let mut current = descendant_idx;
        loop {
            if current == ancestor_idx {
                return true;
            }
            match self.parent_of.get(&current) {
                Some(&parent) => current = parent,
                None => return false,
            }
        }
    }

    /// Collects all descendant handles depth-first for recursive drop.
    fn collect_children_depth_first(&self, index: u32) -> Vec<RawHandle> {
        let mut result = Vec::new();
        let mut stack = Vec::new();

        if let Some(kids) = self.children.get(&index) {
            for &kid in kids {
                stack.push(kid);
            }
        }

        while let Some(idx) = stack.pop() {
            // Get the generation of the child slot.
            if let Some(Slot::Occupied { generation, .. }) = self.slots.get(idx as usize) {
                result.push(RawHandle {
                    index: idx,
                    generation: *generation,
                });
                // Push grandchildren.
                if let Some(grandkids) = self.children.get(&idx) {
                    for &gk in grandkids {
                        stack.push(gk);
                    }
                }
            }
        }

        result
    }
}

impl<T> Default for HandleTable<T> {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Raw Handle
// ═══════════════════════════════════════════════════════════════════════

/// A raw handle: `(index, generation)` pair. This is the internal
/// representation used by `HandleTable` and wrapped by `OwnHandle`/`BorrowHandle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RawHandle {
    /// Slot index in the handle table.
    pub index: u32,
    /// Generation at the time the handle was issued.
    pub generation: Generation,
}

impl RawHandle {
    /// Encodes the handle as a single `u64` for serialization.
    pub fn encode(&self) -> u64 {
        ((self.generation as u64) << 32) | (self.index as u64)
    }

    /// Decodes a `u64` back into a `RawHandle`.
    pub fn decode(encoded: u64) -> Self {
        Self {
            index: (encoded & 0xFFFF_FFFF) as u32,
            generation: (encoded >> 32) as u32,
        }
    }
}

impl fmt::Display for RawHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "handle({}:gen{})", self.index, self.generation)
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W8.4: OwnHandle<T> vs BorrowHandle<T>
// ═══════════════════════════════════════════════════════════════════════

/// An owning handle to a resource of type `T`. The holder is responsible for
/// eventually dropping the resource. In WASI Component Model terms, this
/// corresponds to `own<T>`.
///
/// `OwnHandle<T>` is `Copy` regardless of `T` because it only stores a
/// `u32` index + generation — the actual `T` lives in the `HandleTable`.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct OwnHandle<T> {
    /// The underlying raw handle.
    raw: RawHandle,
    /// Phantom data to associate the handle with type `T`.
    _marker: PhantomData<T>,
}

impl<T> Clone for OwnHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for OwnHandle<T> {}

impl<T> OwnHandle<T> {
    /// Creates a new owning handle from a raw handle.
    pub fn new(raw: RawHandle) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    /// Returns the underlying raw handle.
    pub fn raw(&self) -> RawHandle {
        self.raw
    }

    /// Returns the slot index.
    pub fn index(&self) -> u32 {
        self.raw.index
    }

    /// Returns the generation.
    pub fn generation(&self) -> Generation {
        self.raw.generation
    }
}

impl<T> fmt::Display for OwnHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "own<{}>({}:gen{})",
            std::any::type_name::<T>(),
            self.raw.index,
            self.raw.generation
        )
    }
}

/// A borrowed handle to a resource of type `T`. The holder must NOT drop the
/// resource — it can only read/use it. In WASI Component Model terms, this
/// corresponds to `borrow<T>`.
///
/// `BorrowHandle<T>` is `Copy` regardless of `T` — it is a lightweight
/// reference token, not an owned value.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct BorrowHandle<T> {
    /// The underlying raw handle.
    raw: RawHandle,
    /// Phantom data to associate the handle with type `T`.
    _marker: PhantomData<T>,
}

impl<T> Clone for BorrowHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for BorrowHandle<T> {}

impl<T> BorrowHandle<T> {
    /// Creates a new borrowed handle from a raw handle.
    pub fn new(raw: RawHandle) -> Self {
        Self {
            raw,
            _marker: PhantomData,
        }
    }

    /// Returns the underlying raw handle.
    pub fn raw(&self) -> RawHandle {
        self.raw
    }

    /// Returns the slot index.
    pub fn index(&self) -> u32 {
        self.raw.index
    }

    /// Returns the generation.
    pub fn generation(&self) -> Generation {
        self.raw.generation
    }
}

impl<T> fmt::Display for BorrowHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "borrow<{}>({}:gen{})",
            std::any::type_name::<T>(),
            self.raw.index,
            self.raw.generation
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W8.3: BorrowGuard — RAII borrow tracking
// ═══════════════════════════════════════════════════════════════════════

/// RAII guard that holds an active borrow on a resource handle. When the
/// guard is dropped (goes out of scope), the borrow is automatically released.
///
/// This prevents a resource from being freed while any `BorrowGuard` exists
/// for it.
#[derive(Debug)]
pub struct BorrowGuard<'a, T> {
    /// Reference to the table that owns the resource.
    table: &'a mut HandleTable<T>,
    /// The raw handle being borrowed.
    handle: RawHandle,
    /// Whether the borrow has already been manually released.
    released: bool,
}

impl<'a, T> BorrowGuard<'a, T> {
    /// Creates a new borrow guard, incrementing the borrow count.
    ///
    /// Returns an error if the handle is invalid or stale.
    pub fn acquire(table: &'a mut HandleTable<T>, handle: RawHandle) -> Result<Self, HandleError> {
        table.borrow(handle)?;
        Ok(Self {
            table,
            handle,
            released: false,
        })
    }

    /// Returns the raw handle this guard is borrowing.
    pub fn handle(&self) -> RawHandle {
        self.handle
    }

    /// Returns an immutable reference to the borrowed value.
    pub fn get(&self) -> Result<&T, HandleError> {
        self.table.get(self.handle)
    }

    /// Manually releases the borrow before the guard is dropped.
    pub fn release(&mut self) -> Result<(), HandleError> {
        if !self.released {
            self.released = true;
            self.table.release_borrow(self.handle)?;
        }
        Ok(())
    }
}

impl<T> Drop for BorrowGuard<'_, T> {
    fn drop(&mut self) {
        if !self.released {
            self.released = true;
            // Best-effort release — ignore errors during drop.
            let _ = self.table.release_borrow(self.handle);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W8.5–W8.7: Resource Definition — constructor, methods, statics
// ═══════════════════════════════════════════════════════════════════════

/// A method signature for a resource.
#[derive(Debug, Clone)]
pub struct ResourceMethod {
    /// Method name (e.g., `"read"`, `"write"`).
    pub name: String,
    /// Number of parameters (excluding `self`).
    pub param_count: usize,
    /// Whether this is a static method (no `self` parameter).
    pub is_static: bool,
}

/// A resource type definition that can construct instances, dispatch instance
/// methods, and dispatch static (factory) methods.
///
/// Resources are stored in a `HandleTable` and accessed via `OwnHandle`/`BorrowHandle`.
#[derive(Debug)]
pub struct ResourceDef<T> {
    /// Human-readable resource type name (e.g., `"File"`, `"TcpSocket"`).
    pub name: String,
    /// The handle table holding all live instances of this resource.
    pub table: HandleTable<T>,
    /// Registered instance methods (name → method descriptor).
    methods: HashMap<String, ResourceMethod>,
    /// Registered static methods (name → method descriptor).
    statics: HashMap<String, ResourceMethod>,
}

impl<T> ResourceDef<T> {
    /// Creates a new resource definition with the given type name.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            table: HandleTable::new(),
            methods: HashMap::new(),
            statics: HashMap::new(),
        }
    }

    /// Registers an instance method on this resource type.
    pub fn register_method(&mut self, name: &str, param_count: usize) {
        self.methods.insert(
            name.to_string(),
            ResourceMethod {
                name: name.to_string(),
                param_count,
                is_static: false,
            },
        );
    }

    /// Registers a static method on this resource type.
    pub fn register_static(&mut self, name: &str, param_count: usize) {
        self.statics.insert(
            name.to_string(),
            ResourceMethod {
                name: name.to_string(),
                param_count,
                is_static: true,
            },
        );
    }

    /// Looks up an instance method by name.
    pub fn get_method(&self, name: &str) -> Option<&ResourceMethod> {
        self.methods.get(name)
    }

    /// Looks up a static method by name.
    pub fn get_static(&self, name: &str) -> Option<&ResourceMethod> {
        self.statics.get(name)
    }

    /// Lists all registered instance method names.
    pub fn method_names(&self) -> Vec<&str> {
        self.methods.keys().map(|s| s.as_str()).collect()
    }

    /// Lists all registered static method names.
    pub fn static_names(&self) -> Vec<&str> {
        self.statics.keys().map(|s| s.as_str()).collect()
    }

    // ── W8.5: Resource Constructor ──

    /// Constructs a new resource instance, allocating a handle.
    pub fn construct(&mut self, value: T) -> Result<OwnHandle<T>, HandleError> {
        let raw = self.table.alloc(value)?;
        Ok(OwnHandle::new(raw))
    }

    // ── W8.2: Resource Drop ──

    /// Drops a resource by its owning handle, running cleanup.
    pub fn drop_resource(&mut self, handle: OwnHandle<T>) -> Result<T, HandleError> {
        self.table.free(handle.raw())
    }

    /// Drops a resource and all its nested children (LIFO order).
    pub fn drop_recursive(&mut self, handle: OwnHandle<T>) -> Result<Vec<T>, HandleError> {
        self.table.free_recursive(handle.raw())
    }

    // ── W8.6: Instance Method Dispatch ──

    /// Checks if an instance method call is valid (method exists and handle is live).
    pub fn validate_method_call(
        &self,
        handle: OwnHandle<T>,
        method_name: &str,
    ) -> Result<&ResourceMethod, HandleError> {
        self.table.validate_handle(handle.raw())?;
        self.methods
            .get(method_name)
            .ok_or(HandleError::InvalidIndex(handle.index()))
    }

    /// Gets an immutable reference to the resource value for method dispatch.
    pub fn get_resource(&self, handle: OwnHandle<T>) -> Result<&T, HandleError> {
        self.table.get(handle.raw())
    }

    /// Gets a mutable reference to the resource value for method dispatch.
    pub fn get_resource_mut(&mut self, handle: OwnHandle<T>) -> Result<&mut T, HandleError> {
        self.table.get_mut(handle.raw())
    }

    // ── W8.3: Borrow from OwnHandle ──

    /// Creates a `BorrowHandle` from an `OwnHandle`, incrementing the borrow count.
    pub fn borrow_handle(&mut self, handle: OwnHandle<T>) -> Result<BorrowHandle<T>, HandleError> {
        self.table.borrow(handle.raw())?;
        Ok(BorrowHandle::new(handle.raw()))
    }

    /// Releases a borrow obtained via `borrow_handle`.
    pub fn release_borrow(&mut self, handle: BorrowHandle<T>) -> Result<(), HandleError> {
        self.table.release_borrow(handle.raw())
    }

    /// Gets an immutable reference via a borrowed handle.
    pub fn get_borrowed(&self, handle: BorrowHandle<T>) -> Result<&T, HandleError> {
        self.table.get(handle.raw())
    }

    // ── W8.8: Nested Resources ──

    /// Registers a child resource under a parent. When the parent is dropped
    /// via `drop_recursive`, the child will also be dropped.
    pub fn add_child(
        &mut self,
        parent: OwnHandle<T>,
        child: OwnHandle<T>,
    ) -> Result<(), HandleError> {
        self.table.add_child(parent.raw(), child.raw())
    }
}

// ═══════════════════════════════════════════════════════════════════════
// W8.9: Resource Collections — batch operations on Vec<OwnHandle<T>>
// ═══════════════════════════════════════════════════════════════════════

/// Batch-drops a vector of owned handles from a resource definition.
///
/// Drops are performed in reverse order (LIFO) to respect construction order.
/// Returns all successfully freed values and the first error encountered (if any).
pub fn batch_drop<T>(
    def: &mut ResourceDef<T>,
    handles: Vec<OwnHandle<T>>,
) -> (Vec<T>, Option<HandleError>) {
    let mut freed = Vec::new();
    let mut first_error = None;

    // Drop in reverse order (LIFO).
    for handle in handles.into_iter().rev() {
        match def.drop_resource(handle) {
            Ok(val) => freed.push(val),
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    (freed, first_error)
}

/// Batch-drops with recursive child cleanup.
pub fn batch_drop_recursive<T>(
    def: &mut ResourceDef<T>,
    handles: Vec<OwnHandle<T>>,
) -> (Vec<T>, Option<HandleError>) {
    let mut freed = Vec::new();
    let mut first_error = None;

    for handle in handles.into_iter().rev() {
        match def.drop_recursive(handle) {
            Ok(vals) => freed.extend(vals),
            Err(e) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    (freed, first_error)
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── W8.1: Handle Table — alloc/free/get ──

    #[test]
    fn w8_1_alloc_and_get() {
        let mut table: HandleTable<String> = HandleTable::new();
        let h = table.alloc("hello".to_string()).unwrap();
        assert_eq!(h.index, 0);
        assert_eq!(h.generation, 0);
        assert_eq!(table.get(h).unwrap(), "hello");
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn w8_1_alloc_multiple_and_free() {
        let mut table: HandleTable<i32> = HandleTable::new();
        let h0 = table.alloc(10).unwrap();
        let h1 = table.alloc(20).unwrap();
        let h2 = table.alloc(30).unwrap();

        assert_eq!(table.len(), 3);
        assert_eq!(*table.get(h1).unwrap(), 20);

        // Free middle slot.
        let freed = table.free(h1).unwrap();
        assert_eq!(freed, 20);
        assert_eq!(table.len(), 2);

        // Reuse freed slot.
        let h3 = table.alloc(40).unwrap();
        assert_eq!(h3.index, 1); // reused index
        assert_eq!(h3.generation, 1); // bumped generation
        assert_eq!(*table.get(h3).unwrap(), 40);

        // Old handle h1 is now stale.
        assert_eq!(
            table.get(h1).unwrap_err(),
            HandleError::StaleHandle {
                index: 1,
                expected: 1,
                actual: 0
            }
        );

        // h0 and h2 still valid.
        assert_eq!(*table.get(h0).unwrap(), 10);
        assert_eq!(*table.get(h2).unwrap(), 30);
    }

    #[test]
    fn w8_1_stale_handle_detection() {
        let mut table: HandleTable<&str> = HandleTable::new();
        let h = table.alloc("alive").unwrap();
        table.free(h).unwrap();

        // Handle is now stale.
        let err = table.get(h).unwrap_err();
        assert!(matches!(err, HandleError::StaleHandle { .. }));
    }

    #[test]
    fn w8_1_invalid_index() {
        let table: HandleTable<u32> = HandleTable::new();
        let fake = RawHandle {
            index: 999,
            generation: 0,
        };
        assert_eq!(table.get(fake).unwrap_err(), HandleError::InvalidIndex(999));
    }

    // ── W8.2: Drop Protocol ──

    #[test]
    fn w8_2_drop_callback_invoked() {
        use std::sync::atomic::{AtomicU32, Ordering};

        static DROP_COUNT: AtomicU32 = AtomicU32::new(0);

        fn on_drop(_val: &String) {
            DROP_COUNT.fetch_add(1, Ordering::Relaxed);
        }

        DROP_COUNT.store(0, Ordering::Relaxed);

        let mut table: HandleTable<String> = HandleTable::new();
        table.set_drop_callback(on_drop);

        let h1 = table.alloc("a".to_string()).unwrap();
        let h2 = table.alloc("b".to_string()).unwrap();

        table.free(h1).unwrap();
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 1);

        table.free(h2).unwrap();
        assert_eq!(DROP_COUNT.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn w8_2_auto_drop_via_resource_def() {
        let mut res: ResourceDef<String> = ResourceDef::new("File");
        let h = res.construct("data.txt".to_string()).unwrap();
        assert_eq!(res.table.len(), 1);
        res.drop_resource(h).unwrap();
        assert_eq!(res.table.len(), 0);
    }

    // ── W8.3: Borrow Semantics ──

    #[test]
    fn w8_3_borrow_prevents_drop() {
        let mut table: HandleTable<i32> = HandleTable::new();
        let h = table.alloc(42).unwrap();

        // Borrow the handle.
        table.borrow(h).unwrap();
        assert_eq!(table.borrow_count(h).unwrap(), 1);

        // Cannot free while borrowed.
        let err = table.free(h).unwrap_err();
        assert_eq!(err, HandleError::ActiveBorrows { index: 0, count: 1 });

        // Release borrow, then free succeeds.
        table.release_borrow(h).unwrap();
        assert_eq!(table.borrow_count(h).unwrap(), 0);
        assert_eq!(table.free(h).unwrap(), 42);
    }

    #[test]
    fn w8_3_borrow_guard_raii() {
        let mut table: HandleTable<String> = HandleTable::new();
        let h = table.alloc("guarded".to_string()).unwrap();

        // Scope-limited borrow via BorrowGuard.
        {
            let guard = BorrowGuard::acquire(&mut table, h).unwrap();
            assert_eq!(guard.get().unwrap(), "guarded");
            assert_eq!(guard.handle(), h);
            // guard drops here → borrow released.
        }

        // After guard drops, free should succeed.
        assert_eq!(table.borrow_count(h).unwrap(), 0);
        table.free(h).unwrap();
    }

    // ── W8.4: OwnHandle vs BorrowHandle ──

    #[test]
    fn w8_4_own_vs_borrow_distinction() {
        let raw = RawHandle {
            index: 5,
            generation: 2,
        };

        let own: OwnHandle<String> = OwnHandle::new(raw);
        let borrowed: BorrowHandle<String> = BorrowHandle::new(raw);

        // Both wrap the same raw handle but are different types.
        assert_eq!(own.raw(), raw);
        assert_eq!(borrowed.raw(), raw);
        assert_eq!(own.index(), 5);
        assert_eq!(borrowed.generation(), 2);

        // Display includes type info.
        let own_str = format!("{own}");
        assert!(own_str.contains("own<"));
        assert!(own_str.contains("5:gen2"));

        let borrow_str = format!("{borrowed}");
        assert!(borrow_str.contains("borrow<"));
    }

    #[test]
    fn w8_4_borrow_from_own_handle() {
        let mut res: ResourceDef<i32> = ResourceDef::new("Counter");
        let own = res.construct(100).unwrap();

        // Create a borrow from the own handle.
        let borrowed = res.borrow_handle(own).unwrap();
        assert_eq!(*res.get_borrowed(borrowed).unwrap(), 100);

        // Cannot drop while borrowed.
        let err = res.drop_resource(own).unwrap_err();
        assert!(matches!(err, HandleError::ActiveBorrows { .. }));

        // Release borrow.
        res.release_borrow(borrowed).unwrap();
        res.drop_resource(own).unwrap();
    }

    // ── W8.5: Resource Constructor ──

    #[test]
    fn w8_5_construct_returns_own_handle() {
        let mut res: ResourceDef<Vec<u8>> = ResourceDef::new("Buffer");
        let h = res.construct(vec![1, 2, 3]).unwrap();
        assert_eq!(h.index(), 0);
        assert_eq!(h.generation(), 0);
        assert_eq!(res.get_resource(h).unwrap(), &vec![1, 2, 3]);
    }

    // ── W8.6: Resource Instance Methods ──

    #[test]
    fn w8_6_method_registration_and_dispatch() {
        let mut res: ResourceDef<String> = ResourceDef::new("File");
        res.register_method("read", 1);
        res.register_method("write", 2);

        assert!(res.get_method("read").is_some());
        assert_eq!(res.get_method("read").unwrap().param_count, 1);
        assert!(!res.get_method("read").unwrap().is_static);
        assert!(res.get_method("write").is_some());
        assert!(res.get_method("close").is_none());

        let names = res.method_names();
        assert!(names.contains(&"read"));
        assert!(names.contains(&"write"));

        // Validate method call on a live handle.
        let h = res.construct("test.fj".to_string()).unwrap();
        assert!(res.validate_method_call(h, "read").is_ok());

        // Mutate through handle.
        res.get_resource_mut(h).unwrap().push_str(".bak");
        assert_eq!(res.get_resource(h).unwrap(), "test.fj.bak");
    }

    // ── W8.7: Static Methods ──

    #[test]
    fn w8_7_static_method_factory_pattern() {
        let mut res: ResourceDef<String> = ResourceDef::new("File");
        res.register_static("open", 1);
        res.register_static("create", 1);

        let open = res.get_static("open").unwrap();
        assert!(open.is_static);
        assert_eq!(open.param_count, 1);

        let static_names = res.static_names();
        assert!(static_names.contains(&"open"));
        assert!(static_names.contains(&"create"));
        assert!(res.get_static("delete").is_none());

        // Simulate factory: static method constructs a resource.
        let h = res.construct("/tmp/out.fj".to_string()).unwrap();
        assert_eq!(res.get_resource(h).unwrap(), "/tmp/out.fj");
    }

    // ── W8.8: Nested Resources — LIFO Drop ──

    #[test]
    fn w8_8_nested_resource_lifo_drop() {
        let mut res: ResourceDef<String> = ResourceDef::new("Node");

        let parent = res.construct("parent".to_string()).unwrap();
        let child1 = res.construct("child1".to_string()).unwrap();
        let child2 = res.construct("child2".to_string()).unwrap();
        let grandchild = res.construct("grandchild".to_string()).unwrap();

        // Build tree: parent → [child1, child2], child1 → [grandchild].
        res.add_child(parent, child1).unwrap();
        res.add_child(parent, child2).unwrap();
        res.add_child(child1, grandchild).unwrap();

        assert_eq!(res.table.len(), 4);

        // Recursive drop: should free grandchild, child2, child1, parent.
        let freed = res.drop_recursive(parent).unwrap();
        assert_eq!(res.table.len(), 0);

        // Parent is last in the freed list.
        assert_eq!(freed.last().unwrap(), "parent");
        // All 4 values freed.
        assert_eq!(freed.len(), 4);
        // Verify children were freed before parent (LIFO).
        let parent_pos = freed.iter().position(|v| v == "parent").unwrap();
        let child1_pos = freed.iter().position(|v| v == "child1").unwrap();
        let grandchild_pos = freed.iter().position(|v| v == "grandchild").unwrap();
        assert!(grandchild_pos < child1_pos);
        assert!(child1_pos < parent_pos);
    }

    #[test]
    fn w8_8_cycle_detection() {
        let mut res: ResourceDef<i32> = ResourceDef::new("Graph");

        let a = res.construct(1).unwrap();
        let b = res.construct(2).unwrap();

        res.add_child(a, b).unwrap();

        // Adding a as child of b would create a cycle.
        let err = res.add_child(b, a).unwrap_err();
        assert!(matches!(err, HandleError::CycleDetected { .. }));
    }

    // ── W8.9: Resource in Collections — batch drop ──

    #[test]
    fn w8_9_batch_drop_collection() {
        let mut res: ResourceDef<i32> = ResourceDef::new("Item");

        let mut handles = Vec::new();
        for i in 0..5 {
            handles.push(res.construct(i * 10).unwrap());
        }
        assert_eq!(res.table.len(), 5);

        let (freed, err) = batch_drop(&mut res, handles);
        assert!(err.is_none());
        assert_eq!(freed.len(), 5);
        assert_eq!(res.table.len(), 0);

        // Values should be in reverse order (LIFO).
        assert_eq!(freed[0], 40);
        assert_eq!(freed[4], 0);
    }

    #[test]
    fn w8_9_batch_drop_recursive_with_children() {
        let mut res: ResourceDef<String> = ResourceDef::new("Dir");

        let dir1 = res.construct("dir1".to_string()).unwrap();
        let file1 = res.construct("dir1/a.fj".to_string()).unwrap();
        res.add_child(dir1, file1).unwrap();

        let dir2 = res.construct("dir2".to_string()).unwrap();
        let file2 = res.construct("dir2/b.fj".to_string()).unwrap();
        res.add_child(dir2, file2).unwrap();

        assert_eq!(res.table.len(), 4);

        let (freed, err) = batch_drop_recursive(&mut res, vec![dir1, dir2]);
        assert!(err.is_none());
        assert_eq!(freed.len(), 4);
        assert_eq!(res.table.len(), 0);
    }

    // ── W8.10: Comprehensive edge-case tests ──

    #[test]
    fn w8_10_raw_handle_encode_decode_roundtrip() {
        let original = RawHandle {
            index: 42,
            generation: 7,
        };
        let encoded = original.encode();
        let decoded = RawHandle::decode(encoded);
        assert_eq!(original, decoded);
    }

    #[test]
    fn w8_10_double_free_returns_error() {
        let mut table: HandleTable<i32> = HandleTable::new();
        let h = table.alloc(99).unwrap();
        table.free(h).unwrap();
        let err = table.free(h).unwrap_err();
        assert!(matches!(err, HandleError::StaleHandle { .. }));
    }

    #[test]
    fn w8_10_multiple_borrows_tracked() {
        let mut table: HandleTable<i32> = HandleTable::new();
        let h = table.alloc(1).unwrap();

        table.borrow(h).unwrap();
        table.borrow(h).unwrap();
        table.borrow(h).unwrap();
        assert_eq!(table.borrow_count(h).unwrap(), 3);

        // Cannot free with 3 borrows.
        assert!(matches!(
            table.free(h).unwrap_err(),
            HandleError::ActiveBorrows { count: 3, .. }
        ));

        table.release_borrow(h).unwrap();
        table.release_borrow(h).unwrap();
        table.release_borrow(h).unwrap();
        assert_eq!(table.borrow_count(h).unwrap(), 0);
        table.free(h).unwrap();
    }

    #[test]
    fn w8_10_get_mut_through_handle() {
        let mut table: HandleTable<Vec<i32>> = HandleTable::new();
        let h = table.alloc(vec![1, 2]).unwrap();

        table.get_mut(h).unwrap().push(3);
        assert_eq!(table.get(h).unwrap(), &vec![1, 2, 3]);
    }

    #[test]
    fn w8_10_handle_table_with_capacity() {
        let table: HandleTable<u8> = HandleTable::with_capacity(64);
        assert!(table.is_empty());
        assert_eq!(table.len(), 0);
        assert_eq!(table.capacity(), 0); // capacity is for Vec pre-alloc, slots are added lazily
    }

    #[test]
    fn w8_10_handle_error_display() {
        let e1 = HandleError::InvalidIndex(42);
        assert_eq!(e1.to_string(), "invalid handle index: 42");

        let e2 = HandleError::StaleHandle {
            index: 1,
            expected: 3,
            actual: 2,
        };
        assert!(e2.to_string().contains("stale handle"));

        let e3 = HandleError::ActiveBorrows { index: 0, count: 2 };
        assert!(e3.to_string().contains("2 active borrow(s)"));

        let e4 = HandleError::TableFull;
        assert_eq!(e4.to_string(), "handle table is full");

        let e5 = HandleError::AlreadyFreed(5);
        assert!(e5.to_string().contains("already freed"));

        let e6 = HandleError::CycleDetected {
            parent: 0,
            child: 1,
        };
        assert!(e6.to_string().contains("cycle detected"));
    }

    #[test]
    fn w8_10_raw_handle_display() {
        let h = RawHandle {
            index: 3,
            generation: 1,
        };
        assert_eq!(h.to_string(), "handle(3:gen1)");
    }

    #[test]
    fn w8_10_slot_reuse_across_generations() {
        let mut table: HandleTable<&str> = HandleTable::new();

        // Allocate and free the same slot multiple times.
        let h0 = table.alloc("gen0").unwrap();
        assert_eq!(h0.generation, 0);
        table.free(h0).unwrap();

        let h1 = table.alloc("gen1").unwrap();
        assert_eq!(h1.index, 0); // same slot
        assert_eq!(h1.generation, 1); // bumped generation
        table.free(h1).unwrap();

        let h2 = table.alloc("gen2").unwrap();
        assert_eq!(h2.index, 0);
        assert_eq!(h2.generation, 2);

        // All old handles are stale.
        assert!(table.get(h0).is_err());
        assert!(table.get(h1).is_err());
        assert_eq!(table.get(h2).unwrap(), &"gen2");
    }
}

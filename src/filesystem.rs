//! W3: WASI P2 Filesystem Interface — `wasi:filesystem/types`.
//!
//! Implements WASI Preview 2 filesystem operations:
//! - Descriptor, DirectoryEntry, Filestat types (W3.1)
//! - open-at, read-via-stream, write-via-stream (W3.2–W3.4)
//! - stat/stat-at, readdir (W3.5–W3.6)
//! - path-create-directory, unlink/remove, rename (W3.7–W3.9)
//!
//! These types and operations form the guest-side API that maps to WASI P2
//! host calls via the canonical ABI.

use std::collections::HashMap;
use std::fmt;

// ═══════════════════════════════════════════════════════════════════════
// W3.1: Filesystem Types
// ═══════════════════════════════════════════════════════════════════════

/// A file descriptor handle in WASI P2.
pub type Descriptor = u32;

/// File types in WASI P2 filesystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Unknown,
    BlockDevice,
    CharacterDevice,
    Directory,
    RegularFile,
    SymbolicLink,
    SocketStream,
    SocketDgram,
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => write!(f, "unknown"),
            Self::BlockDevice => write!(f, "block-device"),
            Self::CharacterDevice => write!(f, "character-device"),
            Self::Directory => write!(f, "directory"),
            Self::RegularFile => write!(f, "regular-file"),
            Self::SymbolicLink => write!(f, "symbolic-link"),
            Self::SocketStream => write!(f, "socket-stream"),
            Self::SocketDgram => write!(f, "socket-dgram"),
        }
    }
}

/// File stat information.
#[derive(Debug, Clone, PartialEq)]
pub struct FileStat {
    /// Device number.
    pub device: u64,
    /// Inode number.
    pub inode: u64,
    /// File type.
    pub filetype: FileType,
    /// Number of hard links.
    pub nlink: u64,
    /// File size in bytes.
    pub size: u64,
    /// Last data access timestamp (nanoseconds since epoch).
    pub data_access_timestamp: u64,
    /// Last data modification timestamp.
    pub data_modification_timestamp: u64,
    /// Last status change timestamp.
    pub status_change_timestamp: u64,
}

/// A directory entry.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectoryEntry {
    /// File type.
    pub filetype: FileType,
    /// Entry name.
    pub name: String,
}

/// Filesystem open flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenFlags {
    /// Create file if not exists.
    pub create: bool,
    /// Fail if not a directory.
    pub directory: bool,
    /// Fail if file exists (with create).
    pub exclusive: bool,
    /// Truncate file to zero length.
    pub truncate: bool,
}

/// Descriptor flags (read/write permissions).
#[derive(Debug, Clone, Copy, Default)]
pub struct DescriptorFlags {
    /// Allow reads.
    pub read: bool,
    /// Allow writes.
    pub write: bool,
    /// Synchronous data integrity.
    pub sync: bool,
    /// Synchronous data integrity (data only).
    pub dsync: bool,
    /// Non-blocking I/O.
    pub nonblock: bool,
    /// Append-only writes.
    pub append: bool,
}

/// Filesystem error type.
#[derive(Debug, Clone, PartialEq)]
pub enum FsError {
    /// Permission denied.
    Access,
    /// Would block (non-blocking I/O).
    WouldBlock,
    /// Resource already exists.
    Exist,
    /// Invalid argument.
    Invalid,
    /// Too many open files.
    Nfile,
    /// File not found.
    NoEntry,
    /// Not a directory.
    NotDir,
    /// Directory not empty.
    NotEmpty,
    /// Not supported.
    NotSupported,
    /// I/O error.
    Io,
    /// File is too large.
    Fbig,
    /// No space left.
    NoSpace,
    /// Is a directory.
    IsDir,
    /// Bad descriptor.
    BadDescriptor,
}

impl fmt::Display for FsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Access => write!(f, "access denied"),
            Self::WouldBlock => write!(f, "would block"),
            Self::Exist => write!(f, "already exists"),
            Self::Invalid => write!(f, "invalid argument"),
            Self::Nfile => write!(f, "too many open files"),
            Self::NoEntry => write!(f, "no such file or directory"),
            Self::NotDir => write!(f, "not a directory"),
            Self::NotEmpty => write!(f, "directory not empty"),
            Self::NotSupported => write!(f, "not supported"),
            Self::Io => write!(f, "I/O error"),
            Self::Fbig => write!(f, "file too large"),
            Self::NoSpace => write!(f, "no space left"),
            Self::IsDir => write!(f, "is a directory"),
            Self::BadDescriptor => write!(f, "bad descriptor"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Simulated In-Memory Filesystem
// ═══════════════════════════════════════════════════════════════════════

/// In-memory filesystem node.
#[derive(Debug, Clone)]
enum FsNode {
    File {
        data: Vec<u8>,
        created: u64,
        modified: u64,
    },
    Directory {
        entries: HashMap<String, usize>, // name -> node index
        created: u64,
    },
}

/// Simulated WASI P2 filesystem for testing.
///
/// Provides all WASI P2 filesystem operations backed by an in-memory tree.
#[derive(Debug)]
pub struct WasiFilesystem {
    /// All filesystem nodes.
    nodes: Vec<FsNode>,
    /// Open descriptor table: descriptor -> (node_index, cursor).
    descriptors: HashMap<Descriptor, (usize, u64)>,
    /// Next descriptor ID.
    next_fd: Descriptor,
    /// Inode counter.
    _next_inode: u64,
    /// Timestamp counter (simulated).
    timestamp: u64,
}

impl WasiFilesystem {
    /// Creates a new filesystem with a root directory.
    pub fn new() -> Self {
        let root = FsNode::Directory {
            entries: HashMap::new(),
            created: 1,
        };
        Self {
            nodes: vec![root],
            descriptors: HashMap::new(),
            next_fd: 3, // 0=stdin, 1=stdout, 2=stderr
            _next_inode: 1,
            timestamp: 1000,
        }
    }

    fn tick(&mut self) -> u64 {
        self.timestamp += 1;
        self.timestamp
    }

    fn alloc_fd(&mut self, node_idx: usize) -> Descriptor {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.descriptors.insert(fd, (node_idx, 0));
        fd
    }

    fn resolve_node(&self, fd: Descriptor) -> Result<usize, FsError> {
        self.descriptors
            .get(&fd)
            .map(|&(idx, _)| idx)
            .ok_or(FsError::BadDescriptor)
    }

    fn resolve_path(&self, dir_idx: usize, path: &str) -> Result<usize, FsError> {
        let mut current = dir_idx;
        for component in path.split('/').filter(|c| !c.is_empty()) {
            match &self.nodes[current] {
                FsNode::Directory { entries, .. } => {
                    current = *entries.get(component).ok_or(FsError::NoEntry)?;
                }
                FsNode::File { .. } => return Err(FsError::NotDir),
            }
        }
        Ok(current)
    }

    // ── W3.2: open-at ──

    /// Opens a file relative to a directory descriptor.
    pub fn open_at(
        &mut self,
        dir_fd: Descriptor,
        path: &str,
        flags: OpenFlags,
        _desc_flags: DescriptorFlags,
    ) -> Result<Descriptor, FsError> {
        let dir_idx = self.resolve_node(dir_fd)?;

        // Try to resolve existing path
        match self.resolve_path(dir_idx, path) {
            Ok(node_idx) => {
                if flags.exclusive {
                    return Err(FsError::Exist);
                }
                if flags.directory {
                    if !matches!(self.nodes[node_idx], FsNode::Directory { .. }) {
                        return Err(FsError::NotDir);
                    }
                }
                if flags.truncate {
                    if let FsNode::File { ref mut data, .. } = self.nodes[node_idx] {
                        data.clear();
                    }
                }
                Ok(self.alloc_fd(node_idx))
            }
            Err(FsError::NoEntry) if flags.create => {
                // Create the file
                let ts = self.tick();
                let node_idx = self.nodes.len();
                if flags.directory {
                    self.nodes.push(FsNode::Directory {
                        entries: HashMap::new(),
                        created: ts,
                    });
                } else {
                    self.nodes.push(FsNode::File {
                        data: Vec::new(),
                        created: ts,
                        modified: ts,
                    });
                }

                // Add to parent directory
                let (parent_idx, name) = self.resolve_parent_and_name(dir_idx, path)?;
                if let FsNode::Directory {
                    ref mut entries, ..
                } = self.nodes[parent_idx]
                {
                    entries.insert(name, node_idx);
                }

                Ok(self.alloc_fd(node_idx))
            }
            Err(e) => Err(e),
        }
    }

    fn resolve_parent_and_name(
        &self,
        dir_idx: usize,
        path: &str,
    ) -> Result<(usize, String), FsError> {
        let parts: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
        if parts.is_empty() {
            return Err(FsError::Invalid);
        }
        let name = parts
            .last()
            .expect("non-empty after is_empty check")
            .to_string();
        if parts.len() == 1 {
            return Ok((dir_idx, name));
        }
        let parent_path = parts[..parts.len() - 1].join("/");
        let parent_idx = self.resolve_path(dir_idx, &parent_path)?;
        Ok((parent_idx, name))
    }

    // ── W3.3: read-via-stream ──

    /// Reads bytes from a file descriptor.
    pub fn read(&mut self, fd: Descriptor, len: u64) -> Result<Vec<u8>, FsError> {
        let (node_idx, cursor) = *self.descriptors.get(&fd).ok_or(FsError::BadDescriptor)?;
        match &self.nodes[node_idx] {
            FsNode::File { data, .. } => {
                let start = cursor as usize;
                let end = (start + len as usize).min(data.len());
                let bytes = data[start..end].to_vec();
                // Update cursor
                if let Some(entry) = self.descriptors.get_mut(&fd) {
                    entry.1 = end as u64;
                }
                Ok(bytes)
            }
            FsNode::Directory { .. } => Err(FsError::IsDir),
        }
    }

    // ── W3.4: write-via-stream ──

    /// Writes bytes to a file descriptor.
    pub fn write(&mut self, fd: Descriptor, bytes: &[u8]) -> Result<u64, FsError> {
        let (node_idx, cursor) = *self.descriptors.get(&fd).ok_or(FsError::BadDescriptor)?;
        let ts = self.tick();
        match &mut self.nodes[node_idx] {
            FsNode::File { data, modified, .. } => {
                let start = cursor as usize;
                // Extend if necessary
                if start + bytes.len() > data.len() {
                    data.resize(start + bytes.len(), 0);
                }
                data[start..start + bytes.len()].copy_from_slice(bytes);
                *modified = ts;
                // Update cursor
                if let Some(entry) = self.descriptors.get_mut(&fd) {
                    entry.1 = (start + bytes.len()) as u64;
                }
                Ok(bytes.len() as u64)
            }
            FsNode::Directory { .. } => Err(FsError::IsDir),
        }
    }

    // ── W3.5: stat ──

    /// Gets file stat information for a descriptor.
    pub fn stat(&self, fd: Descriptor) -> Result<FileStat, FsError> {
        let node_idx = self.resolve_node(fd)?;
        self.stat_node(node_idx)
    }

    /// Gets file stat for a path relative to a directory.
    pub fn stat_at(&self, dir_fd: Descriptor, path: &str) -> Result<FileStat, FsError> {
        let dir_idx = self.resolve_node(dir_fd)?;
        let node_idx = self.resolve_path(dir_idx, path)?;
        self.stat_node(node_idx)
    }

    fn stat_node(&self, node_idx: usize) -> Result<FileStat, FsError> {
        match &self.nodes[node_idx] {
            FsNode::File {
                data,
                created,
                modified,
            } => Ok(FileStat {
                device: 0,
                inode: node_idx as u64,
                filetype: FileType::RegularFile,
                nlink: 1,
                size: data.len() as u64,
                data_access_timestamp: *modified,
                data_modification_timestamp: *modified,
                status_change_timestamp: *created,
            }),
            FsNode::Directory { created, entries } => Ok(FileStat {
                device: 0,
                inode: node_idx as u64,
                filetype: FileType::Directory,
                nlink: entries.len() as u64 + 2,
                size: 0,
                data_access_timestamp: *created,
                data_modification_timestamp: *created,
                status_change_timestamp: *created,
            }),
        }
    }

    // ── W3.6: readdir ──

    /// Reads directory entries.
    pub fn readdir(&self, dir_fd: Descriptor) -> Result<Vec<DirectoryEntry>, FsError> {
        let node_idx = self.resolve_node(dir_fd)?;
        match &self.nodes[node_idx] {
            FsNode::Directory { entries, .. } => {
                let mut result = Vec::new();
                for (name, &child_idx) in entries {
                    let filetype = match &self.nodes[child_idx] {
                        FsNode::File { .. } => FileType::RegularFile,
                        FsNode::Directory { .. } => FileType::Directory,
                    };
                    result.push(DirectoryEntry {
                        filetype,
                        name: name.clone(),
                    });
                }
                result.sort_by(|a, b| a.name.cmp(&b.name));
                Ok(result)
            }
            FsNode::File { .. } => Err(FsError::NotDir),
        }
    }

    // ── W3.7: path-create-directory ──

    /// Creates a directory (and parents if needed).
    pub fn create_directory_at(&mut self, dir_fd: Descriptor, path: &str) -> Result<(), FsError> {
        let dir_idx = self.resolve_node(dir_fd)?;

        let mut current = dir_idx;
        for component in path.split('/').filter(|c| !c.is_empty()) {
            match self.resolve_path(current, component) {
                Ok(idx) => {
                    if !matches!(self.nodes[idx], FsNode::Directory { .. }) {
                        return Err(FsError::Exist);
                    }
                    current = idx;
                }
                Err(FsError::NoEntry) => {
                    let ts = self.tick();
                    let new_idx = self.nodes.len();
                    self.nodes.push(FsNode::Directory {
                        entries: HashMap::new(),
                        created: ts,
                    });
                    if let FsNode::Directory {
                        ref mut entries, ..
                    } = self.nodes[current]
                    {
                        entries.insert(component.to_string(), new_idx);
                    }
                    current = new_idx;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    // ── W3.8: unlink-file / remove-directory ──

    /// Unlinks a file at the given path.
    pub fn unlink_file_at(&mut self, dir_fd: Descriptor, path: &str) -> Result<(), FsError> {
        let dir_idx = self.resolve_node(dir_fd)?;
        let (parent_idx, name) = self.resolve_parent_and_name(dir_idx, path)?;

        // Check it exists and is a file
        if let FsNode::Directory { ref entries, .. } = self.nodes[parent_idx] {
            let &child_idx = entries.get(&name).ok_or(FsError::NoEntry)?;
            if matches!(self.nodes[child_idx], FsNode::Directory { .. }) {
                return Err(FsError::IsDir);
            }
        } else {
            return Err(FsError::NotDir);
        }

        // Remove from parent
        if let FsNode::Directory {
            ref mut entries, ..
        } = self.nodes[parent_idx]
        {
            entries.remove(&name);
        }
        Ok(())
    }

    /// Removes an empty directory at the given path.
    pub fn remove_directory_at(&mut self, dir_fd: Descriptor, path: &str) -> Result<(), FsError> {
        let dir_idx = self.resolve_node(dir_fd)?;
        let (parent_idx, name) = self.resolve_parent_and_name(dir_idx, path)?;

        // Check it's a directory and is empty
        if let FsNode::Directory { ref entries, .. } = self.nodes[parent_idx] {
            let &child_idx = entries.get(&name).ok_or(FsError::NoEntry)?;
            match &self.nodes[child_idx] {
                FsNode::Directory {
                    entries: child_entries,
                    ..
                } => {
                    if !child_entries.is_empty() {
                        return Err(FsError::NotEmpty);
                    }
                }
                FsNode::File { .. } => return Err(FsError::NotDir),
            }
        } else {
            return Err(FsError::NotDir);
        }

        if let FsNode::Directory {
            ref mut entries, ..
        } = self.nodes[parent_idx]
        {
            entries.remove(&name);
        }
        Ok(())
    }

    // ── W3.9: path-rename ──

    /// Renames a file or directory atomically.
    pub fn rename_at(
        &mut self,
        old_dir_fd: Descriptor,
        old_path: &str,
        new_dir_fd: Descriptor,
        new_path: &str,
    ) -> Result<(), FsError> {
        let old_dir_idx = self.resolve_node(old_dir_fd)?;
        let new_dir_idx = self.resolve_node(new_dir_fd)?;

        let (old_parent, old_name) = self.resolve_parent_and_name(old_dir_idx, old_path)?;
        let (new_parent, new_name) = self.resolve_parent_and_name(new_dir_idx, new_path)?;

        // Get the node index from old parent
        let node_idx = if let FsNode::Directory { ref entries, .. } = self.nodes[old_parent] {
            *entries.get(&old_name).ok_or(FsError::NoEntry)?
        } else {
            return Err(FsError::NotDir);
        };

        // Remove from old parent
        if let FsNode::Directory {
            ref mut entries, ..
        } = self.nodes[old_parent]
        {
            entries.remove(&old_name);
        }

        // Add to new parent
        if let FsNode::Directory {
            ref mut entries, ..
        } = self.nodes[new_parent]
        {
            entries.insert(new_name, node_idx);
        } else {
            return Err(FsError::NotDir);
        }

        Ok(())
    }

    /// Opens the root directory and returns its descriptor.
    pub fn open_root(&mut self) -> Descriptor {
        self.alloc_fd(0)
    }

    /// Closes a descriptor.
    pub fn close(&mut self, fd: Descriptor) -> Result<(), FsError> {
        self.descriptors
            .remove(&fd)
            .map(|_| ())
            .ok_or(FsError::BadDescriptor)
    }
}

impl Default for WasiFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (WasiFilesystem, Descriptor) {
        let mut fs = WasiFilesystem::new();
        let root = fs.open_root();
        (fs, root)
    }

    // ── W3.1: Types ──

    #[test]
    fn w3_1_filetype_display() {
        assert_eq!(FileType::RegularFile.to_string(), "regular-file");
        assert_eq!(FileType::Directory.to_string(), "directory");
        assert_eq!(FileType::SymbolicLink.to_string(), "symbolic-link");
    }

    #[test]
    fn w3_1_fs_error_display() {
        assert_eq!(FsError::NoEntry.to_string(), "no such file or directory");
        assert_eq!(FsError::Access.to_string(), "access denied");
    }

    // ── W3.2: open-at ──

    #[test]
    fn w3_2_open_create_file() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(root, "file.txt", flags, DescriptorFlags::default())
            .unwrap();
        assert!(fd >= 3);
    }

    #[test]
    fn w3_2_open_existing_file() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let _fd1 = fs
            .open_at(root, "file.txt", flags, DescriptorFlags::default())
            .unwrap();
        let fd2 = fs
            .open_at(
                root,
                "file.txt",
                OpenFlags::default(),
                DescriptorFlags::default(),
            )
            .unwrap();
        assert!(fd2 > 0);
    }

    #[test]
    fn w3_2_open_nonexistent_fails() {
        let (mut fs, root) = setup();
        let err = fs
            .open_at(
                root,
                "missing.txt",
                OpenFlags::default(),
                DescriptorFlags::default(),
            )
            .unwrap_err();
        assert_eq!(err, FsError::NoEntry);
    }

    // ── W3.3: read-via-stream ──

    #[test]
    fn w3_3_read_file_in_chunks() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(root, "data.bin", flags, DescriptorFlags::default())
            .unwrap();

        // Write 1MB of data
        let data: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
        fs.write(fd, &data).unwrap();

        // Close and reopen to reset cursor
        fs.close(fd).unwrap();
        let fd2 = fs
            .open_at(
                root,
                "data.bin",
                OpenFlags::default(),
                DescriptorFlags::default(),
            )
            .unwrap();

        // Read in 4KB chunks
        let mut read_data = Vec::new();
        loop {
            let chunk = fs.read(fd2, 4096).unwrap();
            if chunk.is_empty() {
                break;
            }
            read_data.extend_from_slice(&chunk);
        }
        assert_eq!(read_data.len(), 1024 * 1024);
        assert_eq!(&read_data[..10], &data[..10]);
    }

    // ── W3.4: write-via-stream ──

    #[test]
    fn w3_4_write_and_verify() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(root, "hello.txt", flags, DescriptorFlags::default())
            .unwrap();
        let written = fs.write(fd, b"Hello, WASI!").unwrap();
        assert_eq!(written, 12);

        // Reset cursor and read back
        fs.close(fd).unwrap();
        let fd2 = fs
            .open_at(
                root,
                "hello.txt",
                OpenFlags::default(),
                DescriptorFlags::default(),
            )
            .unwrap();
        let data = fs.read(fd2, 100).unwrap();
        assert_eq!(data, b"Hello, WASI!");
    }

    // ── W3.5: stat ──

    #[test]
    fn w3_5_stat_file() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(root, "test.txt", flags, DescriptorFlags::default())
            .unwrap();
        fs.write(fd, b"hello world").unwrap();

        let stat = fs.stat(fd).unwrap();
        assert_eq!(stat.filetype, FileType::RegularFile);
        assert_eq!(stat.size, 11); // "hello world"
        assert_eq!(stat.nlink, 1);
    }

    #[test]
    fn w3_5_stat_at_path() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(root, "data.bin", flags, DescriptorFlags::default())
            .unwrap();
        fs.write(fd, &vec![0u8; 1024]).unwrap();

        let stat = fs.stat_at(root, "data.bin").unwrap();
        assert_eq!(stat.size, 1024);
    }

    // ── W3.6: readdir ──

    #[test]
    fn w3_6_readdir_lists_entries() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        fs.open_at(root, "a.txt", flags, DescriptorFlags::default())
            .unwrap();
        fs.open_at(root, "b.txt", flags, DescriptorFlags::default())
            .unwrap();
        fs.create_directory_at(root, "subdir").unwrap();

        let entries = fs.readdir(root).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[0].filetype, FileType::RegularFile);
        assert_eq!(entries[1].name, "b.txt");
        assert_eq!(entries[2].name, "subdir");
        assert_eq!(entries[2].filetype, FileType::Directory);
    }

    // ── W3.7: path-create-directory ──

    #[test]
    fn w3_7_create_nested_directories() {
        let (mut fs, root) = setup();
        fs.create_directory_at(root, "a/b/c").unwrap();

        let entries = fs.readdir(root).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a");

        // Open 'a' and check 'b' exists
        let a_fd = fs
            .open_at(
                root,
                "a",
                OpenFlags {
                    directory: true,
                    ..Default::default()
                },
                DescriptorFlags::default(),
            )
            .unwrap();
        let a_entries = fs.readdir(a_fd).unwrap();
        assert_eq!(a_entries.len(), 1);
        assert_eq!(a_entries[0].name, "b");
    }

    // ── W3.8: unlink-file / remove-directory ──

    #[test]
    fn w3_8_unlink_file() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        fs.open_at(root, "to_delete.txt", flags, DescriptorFlags::default())
            .unwrap();
        assert_eq!(fs.readdir(root).unwrap().len(), 1);

        fs.unlink_file_at(root, "to_delete.txt").unwrap();
        assert_eq!(fs.readdir(root).unwrap().len(), 0);

        // stat should fail
        let err = fs.stat_at(root, "to_delete.txt").unwrap_err();
        assert_eq!(err, FsError::NoEntry);
    }

    #[test]
    fn w3_8_remove_empty_directory() {
        let (mut fs, root) = setup();
        fs.create_directory_at(root, "empty_dir").unwrap();
        fs.remove_directory_at(root, "empty_dir").unwrap();
        assert_eq!(fs.readdir(root).unwrap().len(), 0);
    }

    #[test]
    fn w3_8_remove_nonempty_fails() {
        let (mut fs, root) = setup();
        fs.create_directory_at(root, "nonempty").unwrap();
        let dir_fd = fs
            .open_at(
                root,
                "nonempty",
                OpenFlags {
                    directory: true,
                    ..Default::default()
                },
                DescriptorFlags::default(),
            )
            .unwrap();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        fs.open_at(dir_fd, "child.txt", flags, DescriptorFlags::default())
            .unwrap();

        let err = fs.remove_directory_at(root, "nonempty").unwrap_err();
        assert_eq!(err, FsError::NotEmpty);
    }

    // ── W3.9: path-rename ──

    #[test]
    fn w3_9_rename_preserves_content() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(root, "old.txt", flags, DescriptorFlags::default())
            .unwrap();
        fs.write(fd, b"content here").unwrap();

        fs.rename_at(root, "old.txt", root, "new.txt").unwrap();

        // Old name gone
        assert!(fs.stat_at(root, "old.txt").is_err());

        // New name has same content
        let new_fd = fs
            .open_at(
                root,
                "new.txt",
                OpenFlags::default(),
                DescriptorFlags::default(),
            )
            .unwrap();
        let data = fs.read(new_fd, 100).unwrap();
        assert_eq!(data, b"content here");
    }

    // ── W3.10: Comprehensive tests ──

    #[test]
    fn w3_10_full_filesystem_workflow() {
        let (mut fs, root) = setup();

        // Create directory structure
        fs.create_directory_at(root, "src").unwrap();
        fs.create_directory_at(root, "src/lib").unwrap();

        // Create files
        let src_fd = fs
            .open_at(
                root,
                "src",
                OpenFlags {
                    directory: true,
                    ..Default::default()
                },
                DescriptorFlags::default(),
            )
            .unwrap();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(src_fd, "main.fj", flags, DescriptorFlags::default())
            .unwrap();
        fs.write(fd, b"fn main() { println(\"hello\") }").unwrap();

        let lib_fd = fs
            .open_at(
                src_fd,
                "lib",
                OpenFlags {
                    directory: true,
                    ..Default::default()
                },
                DescriptorFlags::default(),
            )
            .unwrap();
        let fd2 = fs
            .open_at(lib_fd, "utils.fj", flags, DescriptorFlags::default())
            .unwrap();
        fs.write(fd2, b"pub fn helper() -> i32 { 42 }").unwrap();

        // Verify stat
        let stat = fs.stat_at(src_fd, "main.fj").unwrap();
        assert_eq!(stat.filetype, FileType::RegularFile);
        assert_eq!(stat.size, 30);

        // Readdir
        let entries = fs.readdir(src_fd).unwrap();
        assert_eq!(entries.len(), 2); // lib/ + main.fj

        // Rename
        fs.rename_at(src_fd, "main.fj", src_fd, "app.fj").unwrap();
        let entries = fs.readdir(src_fd).unwrap();
        assert!(entries.iter().any(|e| e.name == "app.fj"));
        assert!(!entries.iter().any(|e| e.name == "main.fj"));
    }

    #[test]
    fn w3_10_descriptor_close_and_reopen() {
        let (mut fs, root) = setup();
        let flags = OpenFlags {
            create: true,
            ..Default::default()
        };
        let fd = fs
            .open_at(root, "test.txt", flags, DescriptorFlags::default())
            .unwrap();
        fs.write(fd, b"hello").unwrap();
        fs.close(fd).unwrap();

        // Re-reading after close should fail
        assert!(fs.read(fd, 10).is_err());

        // But reopening works
        let fd2 = fs
            .open_at(
                root,
                "test.txt",
                OpenFlags::default(),
                DescriptorFlags::default(),
            )
            .unwrap();
        let data = fs.read(fd2, 10).unwrap();
        assert_eq!(data, b"hello");
    }
}

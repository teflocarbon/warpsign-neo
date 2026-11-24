//! An AArch64 machine in Unicorn that runs Apple's Android ADI libraries.
//!
//! The libraries are loaded at fixed addresses and relocated here. What they
//! import from Android's libc (about thirty functions: files, malloc,
//! pthread, dlopen) points at a page of `ret` instructions; a code hook on
//! that page runs our version of the function in Rust, puts the result in
//! x0, and the `ret` goes back to the caller. Files live in memory
//! ([`Files`]), so the libraries never touch the real file system, and what
//! they save (`adi.pb`) can be kept wherever the caller keeps secrets.
//!
//! The list of functions to provide, and that the pthread ones can do
//! nothing, are from Provision (Dadoum, LGPL), which loads the same
//! libraries natively on Linux.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{c_int, c_void};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::elf::{
    R_AARCH64_ABS64, R_AARCH64_GLOB_DAT, R_AARCH64_JUMP_SLOT, R_AARCH64_RELATIVE, SharedObject,
};
use super::unicorn::{self, Engine, Unicorn};

const PAGE: u64 = 0x1000;
const LIBRARY_BASE: u64 = 0x1000_0000;
const LIBRARY_STRIDE: u64 = 0x1000_0000;
const HEAP: u64 = 0x8000_0000;
const HEAP_SIZE: u64 = 32 << 20;
/// One `ret` per function the libraries import
const IMPORTS: u64 = 0xE000_0000;
const IMPORTS_SIZE: u64 = 0x1_0000;
/// Where a call from Rust returns to, so emulation stops there
const RETURN: u64 = 0xE100_0000;
/// errno, and a zeroed area for TPIDR_EL0
const DATA: u64 = 0xE200_0000;
const DATA_SIZE: u64 = 0x4000;
const ERRNO: u64 = DATA;
const TLS: u64 = DATA + 0x1000;
const STACK: u64 = 0xF000_0000;
const STACK_SIZE: u64 = 1 << 20;
/// dlopen's handles: never dereferenced by the libraries
const HANDLES: u64 = 0xD000_0000;

const RET: [u8; 4] = 0xD65F_03C0u32.to_le_bytes();

// Linux errno and open flags, as Android's libc has them
const ENOENT: i32 = 2;
const EBADF: i32 = 9;
const ENOMEM: i32 = 12;
const EEXIST: i32 = 17;
const EISDIR: i32 = 21;
const O_ACCMODE: u64 = 0o3;
const O_CREAT: u64 = 0o100;
const O_EXCL: u64 = 0o200;
const O_TRUNC: u64 = 0o1000;
const O_APPEND: u64 = 0o2000;
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;

/// What the libraries can call, and what we do instead
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Libc {
    Open,
    Close,
    Read,
    Write,
    Fstat,
    Lstat,
    Mkdir,
    Chmod,
    Umask,
    Ftruncate,
    Malloc,
    Free,
    Strncpy,
    Errno,
    Gettimeofday,
    Arc4random,
    SystemPropertyGet,
    Dlopen,
    Dlsym,
    Dlclose,
    /// pthread locks, once and create: one thread, so nothing to do
    Nothing,
}

impl Libc {
    fn named(name: &str) -> Option<Self> {
        Some(match name {
            "open" => Self::Open,
            "close" => Self::Close,
            "read" => Self::Read,
            "write" => Self::Write,
            "fstat" => Self::Fstat,
            "lstat" => Self::Lstat,
            "mkdir" => Self::Mkdir,
            "chmod" => Self::Chmod,
            "umask" => Self::Umask,
            "ftruncate" => Self::Ftruncate,
            "malloc" => Self::Malloc,
            "free" => Self::Free,
            "strncpy" => Self::Strncpy,
            "__errno" => Self::Errno,
            "gettimeofday" => Self::Gettimeofday,
            "arc4random" => Self::Arc4random,
            "__system_property_get" => Self::SystemPropertyGet,
            "dlopen" => Self::Dlopen,
            "dlsym" => Self::Dlsym,
            "dlclose" => Self::Dlclose,
            "pthread_once"
            | "pthread_create"
            | "pthread_mutex_lock"
            | "pthread_mutex_unlock"
            | "pthread_rwlock_init"
            | "pthread_rwlock_destroy"
            | "pthread_rwlock_rdlock"
            | "pthread_rwlock_wrlock"
            | "pthread_rwlock_unlock" => Self::Nothing,
            _ => return None,
        })
    }
}

enum Slot {
    Libc(Libc),
    /// Imported but not provided: calling it is an error
    Missing(String),
}

struct Library {
    name: String,
    base: u64,
    elf: SharedObject,
}

/// The libraries' files, in memory: paths are absolute and normalised
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Files {
    pub files: BTreeMap<String, Vec<u8>>,
    pub directories: BTreeSet<String>,
}

impl Files {
    fn parent_exists(&self, path: &str) -> bool {
        match path.rsplit_once('/') {
            Some(("", _)) => true,
            Some((parent, _)) => self.directories.contains(parent),
            None => false,
        }
    }

    /// Create `path` and the directories above it
    pub fn create_directories(&mut self, path: &str) {
        let mut current = String::new();
        for part in normalise(path).split('/').filter(|p| !p.is_empty()) {
            current.push('/');
            current.push_str(part);
            self.directories.insert(current.clone());
        }
    }
}

pub fn normalise(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    format!("/{}", parts.join("/"))
}

struct OpenFile {
    path: String,
    position: usize,
    directory: bool,
    append: bool,
}

struct Heap {
    next: u64,
    sizes: HashMap<u64, u64>,
    free: HashMap<u64, Vec<u64>>,
}

impl Heap {
    fn allocate(&mut self, size: u64) -> Option<u64> {
        let size = size.max(16).div_ceil(16) * 16;
        if let Some(address) = self.free.get_mut(&size).and_then(Vec::pop) {
            self.sizes.insert(address, size);
            return Some(address);
        }
        if self.next + size > HEAP + HEAP_SIZE {
            return None;
        }
        let address = self.next;
        self.next += size;
        self.sizes.insert(address, size);
        Some(address)
    }

    fn release(&mut self, address: u64) {
        if let Some(size) = self.sizes.remove(&address) {
            self.free.entry(size).or_default().push(address);
        }
    }
}

struct Inner {
    unicorn: Arc<Unicorn>,
    engine: Engine,
    libraries: Vec<Library>,
    slots: Vec<Slot>,
    slot_names: HashMap<String, u64>,
    heap: Heap,
    files: Files,
    open_files: HashMap<i32, OpenFile>,
    next_fd: i32,
    /// Why emulation was stopped from a hook
    failure: Option<String>,
    /// Name of the import last called, for errors
    last_import: Option<String>,
}

/// A machine with Apple's libraries loaded. Not thread-safe: use it from
/// one thread at a time.
pub struct Machine {
    inner: Box<Inner>,
}

// SAFETY: the engine is only used through &mut self, so by one thread at a
// time; Unicorn engines aren't tied to the thread that made them
unsafe impl Send for Machine {}

fn check(unicorn: &Unicorn, err: c_int, what: &str) -> Result<(), String> {
    if err == 0 {
        Ok(())
    } else {
        Err(format!("{what}: {}", unicorn.describe(err)))
    }
}

impl Machine {
    pub fn new(unicorn: Arc<Unicorn>, files: Files) -> Result<Self, String> {
        let mut engine: Engine = std::ptr::null_mut();
        // SAFETY: Unicorn's API, as unicorn.h documents it; the engine is
        // closed in Drop
        let err =
            unsafe { (unicorn.open)(unicorn::UC_ARCH_ARM64, unicorn::UC_MODE_ARM, &mut engine) };
        check(&unicorn, err, "couldn't start Unicorn")?;
        let mut machine = Machine {
            inner: Box::new(Inner {
                unicorn,
                engine,
                libraries: Vec::new(),
                slots: Vec::new(),
                slot_names: HashMap::new(),
                heap: Heap {
                    next: HEAP,
                    sizes: HashMap::new(),
                    free: HashMap::new(),
                },
                files,
                open_files: HashMap::new(),
                next_fd: 3,
                failure: None,
                last_import: None,
            }),
        };
        machine.set_up()?;
        Ok(machine)
    }

    fn set_up(&mut self) -> Result<(), String> {
        let inner = &mut *self.inner;
        for (address, size) in [
            (HEAP, HEAP_SIZE),
            (IMPORTS, IMPORTS_SIZE),
            (RETURN, PAGE),
            (DATA, DATA_SIZE),
            (STACK, STACK_SIZE),
        ] {
            inner.map(address, size)?;
        }
        let rets: Vec<u8> = RET
            .iter()
            .copied()
            .cycle()
            .take(IMPORTS_SIZE as usize)
            .collect();
        inner.write(IMPORTS, &rets)?;
        inner.write(RETURN, &RET)?;
        // Floating point and SIMD on (CPACR_EL1.FPEN), and a TLS area
        inner.set_register(unicorn::REG_CPACR_EL1, 3 << 20)?;
        inner.set_register(unicorn::REG_TPIDR_EL0, TLS)?;

        let user = (&mut *self.inner as *mut Inner).cast::<c_void>();
        let inner = &*self.inner;
        let mut handle = 0;
        // SAFETY: `user` is the boxed Inner, which outlives the engine
        // (Drop closes the engine first); the callbacks have the types
        // unicorn.h gives these hook types
        let err = unsafe {
            (inner.unicorn.hook_add)(
                inner.engine,
                &mut handle,
                unicorn::UC_HOOK_CODE,
                on_import as unicorn::CodeHook as *mut c_void,
                user,
                IMPORTS,
                IMPORTS + IMPORTS_SIZE - 1,
            )
        };
        check(&inner.unicorn, err, "couldn't hook the imports")?;
        // SAFETY: as above
        let err = unsafe {
            (inner.unicorn.hook_add)(
                inner.engine,
                &mut handle,
                unicorn::UC_HOOK_MEM_UNMAPPED,
                on_unmapped as unicorn::MemHook as *mut c_void,
                user,
                1,
                0,
            )
        };
        check(&inner.unicorn, err, "couldn't hook unmapped memory")
    }

    /// Load a library, binding its imports to our libc or to libraries
    /// loaded before it
    pub fn load(&mut self, name: &str, data: &[u8]) -> Result<(), String> {
        let elf = SharedObject::parse(data).map_err(|e| format!("{name}: {e}"))?;
        let inner = &mut *self.inner;
        let base = LIBRARY_BASE + LIBRARY_STRIDE * inner.libraries.len() as u64;
        let (start, end) = elf.extent();
        let start = start / PAGE * PAGE;
        let end = end.div_ceil(PAGE) * PAGE;
        if end - start > LIBRARY_STRIDE {
            return Err(format!("{name} is too big to load"));
        }

        let mut image = vec![0u8; (end - start) as usize];
        for segment in &elf.segments {
            let at = (segment.vaddr - start) as usize;
            let from = segment.offset as usize;
            image[at..at + segment.filesz as usize]
                .copy_from_slice(&data[from..from + segment.filesz as usize]);
        }

        for relocation in &elf.relocations {
            let symbol = elf
                .symbols
                .get(relocation.symbol as usize)
                .ok_or_else(|| format!("{name}: relocation names a missing symbol"))?;
            let resolve = |inner: &mut Inner| match symbol.value {
                Some(value) => base + value,
                None => inner.import(&symbol.name),
            };
            let value = match relocation.kind {
                R_AARCH64_RELATIVE => base.wrapping_add_signed(relocation.addend),
                R_AARCH64_GLOB_DAT | R_AARCH64_ABS64 => {
                    resolve(inner).wrapping_add_signed(relocation.addend)
                }
                R_AARCH64_JUMP_SLOT => resolve(inner),
                kind => return Err(format!("{name}: unsupported relocation type {kind}")),
            };
            let at = relocation
                .offset
                .checked_sub(start)
                .map(|at| at as usize)
                .filter(|at| at + 8 <= image.len())
                .ok_or_else(|| format!("{name}: relocation outside the library"))?;
            image[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }

        inner.map(base + start, end - start)?;
        inner.write(base + start, &image)?;
        inner.libraries.push(Library {
            name: name.to_string(),
            base,
            elf,
        });
        Ok(())
    }

    /// Where an exported function of a loaded library is
    pub fn symbol(&self, library: &str, name: &str) -> Option<u64> {
        let library = self.inner.libraries.iter().find(|l| l.name == library)?;
        library.elf.export(name).map(|value| library.base + value)
    }

    /// Imports neither our libc nor an earlier library provides
    pub fn missing_imports(&self) -> Vec<String> {
        self.inner
            .slots
            .iter()
            .filter_map(|slot| match slot {
                Slot::Missing(name) => Some(name.clone()),
                Slot::Libc(_) => None,
            })
            .collect()
    }

    /// Call the function at `address` with up to eight integer arguments
    pub fn call(&mut self, address: u64, arguments: &[u64]) -> Result<u64, String> {
        if arguments.len() > 8 {
            return Err("more than eight arguments".into());
        }
        let inner: *mut Inner = &mut *self.inner;
        // SAFETY: nothing else uses Inner during this call, except the
        // hooks, through the same pointer, while emu_start runs
        unsafe {
            for (i, &value) in arguments.iter().enumerate() {
                (*inner).set_register(unicorn::REG_X0 + i as c_int, value)?;
            }
            (*inner).set_register(unicorn::REG_SP, STACK + STACK_SIZE - 0x100)?;
            (*inner).set_register(unicorn::REG_X29, 0)?;
            (*inner).set_register(unicorn::REG_X30, RETURN)?;
            (*inner).failure = None;
            (*inner).last_import = None;
            let unicorn = Arc::clone(&(*inner).unicorn);
            let engine = (*inner).engine;
            let err = (unicorn.emu_start)(engine, address, RETURN, 0, 0);
            if let Some(failure) = (*inner).failure.take() {
                return Err(failure);
            }
            if err != 0 {
                let pc = (*inner).register(unicorn::REG_PC).unwrap_or(0);
                let after = (*inner)
                    .last_import
                    .as_deref()
                    .map(|name| format!(", last import called {name}"))
                    .unwrap_or_default();
                return Err(format!(
                    "emulation stopped at {pc:#x}{after}: {}",
                    unicorn.describe(err)
                ));
            }
            (*inner).register(unicorn::REG_X0)
        }
    }

    /// Copy bytes into the machine's heap
    pub fn put(&mut self, bytes: &[u8]) -> Result<u64, String> {
        let address = self
            .inner
            .heap
            .allocate(bytes.len() as u64)
            .ok_or("the machine's heap is full")?;
        self.inner.write(address, bytes)?;
        Ok(address)
    }

    /// Free what `put` returned
    pub fn release(&mut self, address: u64) {
        self.inner.heap.release(address);
    }

    pub fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        self.inner.read(address, length)
    }

    pub fn read_u64(&self, address: u64) -> Result<u64, String> {
        self.inner.read_u64(address)
    }

    pub fn read_u32(&self, address: u64) -> Result<u32, String> {
        let bytes = self.inner.read(address, 4)?;
        Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
    }

    pub fn files(&self) -> &Files {
        &self.inner.files
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        // SAFETY: closing the engine made in `new`, once
        unsafe { (self.inner.unicorn.close)(self.inner.engine) };
    }
}

impl Inner {
    fn map(&self, address: u64, size: u64) -> Result<(), String> {
        // SAFETY: Unicorn's API on our engine
        let err = unsafe {
            (self.unicorn.mem_map)(self.engine, address, size as usize, unicorn::UC_PROT_ALL)
        };
        check(&self.unicorn, err, &format!("couldn't map {address:#x}"))
    }

    fn write(&self, address: u64, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: Unicorn copies `bytes.len()` bytes from `bytes`
        let err = unsafe {
            (self.unicorn.mem_write)(self.engine, address, bytes.as_ptr().cast(), bytes.len())
        };
        check(
            &self.unicorn,
            err,
            &format!("couldn't write at {address:#x}"),
        )
    }

    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        let mut bytes = vec![0u8; length];
        // SAFETY: Unicorn copies `length` bytes into `bytes`
        let err = unsafe {
            (self.unicorn.mem_read)(self.engine, address, bytes.as_mut_ptr().cast(), length)
        };
        check(
            &self.unicorn,
            err,
            &format!("couldn't read at {address:#x}"),
        )?;
        Ok(bytes)
    }

    fn read_u64(&self, address: u64) -> Result<u64, String> {
        let bytes = self.read(address, 8)?;
        Ok(u64::from_le_bytes(bytes.try_into().unwrap()))
    }

    fn read_string(&self, address: u64) -> Result<String, String> {
        let mut bytes = Vec::new();
        let mut at = address;
        loop {
            // Don't read past a page boundary in one go: the next page may
            // not be mapped
            let chunk = (PAGE - at % PAGE).min(256) as usize;
            let part = self.read(at, chunk)?;
            if let Some(end) = part.iter().position(|&b| b == 0) {
                bytes.extend_from_slice(&part[..end]);
                break;
            }
            bytes.extend_from_slice(&part);
            at += chunk as u64;
            if bytes.len() > 4096 {
                return Err("string too long".into());
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn set_register(&self, register: c_int, value: u64) -> Result<(), String> {
        // SAFETY: Unicorn reads a u64 for 64-bit registers
        let err = unsafe {
            (self.unicorn.reg_write)(self.engine, register, (&value as *const u64).cast())
        };
        check(&self.unicorn, err, "couldn't set a register")
    }

    fn register(&self, register: c_int) -> Result<u64, String> {
        let mut value = 0u64;
        // SAFETY: Unicorn writes a u64 for 64-bit registers
        let err = unsafe {
            (self.unicorn.reg_read)(self.engine, register, (&mut value as *mut u64).cast())
        };
        check(&self.unicorn, err, "couldn't read a register")?;
        Ok(value)
    }

    /// The address an import of `name` binds to
    fn import(&mut self, name: &str) -> u64 {
        if let Some(&address) = self.slot_names.get(name) {
            return address;
        }
        if let Some(address) = self
            .libraries
            .iter()
            .find_map(|l| l.elf.export(name).map(|value| l.base + value))
        {
            return address;
        }
        let slot = match Libc::named(name) {
            Some(function) => Slot::Libc(function),
            None => Slot::Missing(name.to_string()),
        };
        let address = IMPORTS + 4 * self.slots.len() as u64;
        self.slots.push(slot);
        self.slot_names.insert(name.to_string(), address);
        address
    }

    fn set_errno(&self, errno: i32) {
        let _ = self.write(ERRNO, &errno.to_le_bytes());
    }

    /// -1 with errno set, as libc reports failure
    fn fail(&self, errno: i32) -> u64 {
        self.set_errno(errno);
        u64::MAX
    }

    fn argument(&self, index: c_int) -> Result<u64, String> {
        self.register(unicorn::REG_X0 + index)
    }

    fn write_stat(&self, address: u64, path: &str) -> Result<u64, String> {
        let (mode, size) = if self.files.directories.contains(path) || path == "/" {
            (S_IFDIR | 0o755, 0)
        } else if let Some(file) = self.files.files.get(path) {
            (S_IFREG | 0o644, file.len() as u64)
        } else {
            return Ok(self.fail(ENOENT));
        };
        // struct stat for Linux on AArch64, 128 bytes
        let mut stat = [0u8; 128];
        stat[8..16].copy_from_slice(&1u64.to_le_bytes()); // st_ino
        stat[16..20].copy_from_slice(&mode.to_le_bytes());
        stat[20..24].copy_from_slice(&1u32.to_le_bytes()); // st_nlink
        stat[48..56].copy_from_slice(&size.to_le_bytes());
        stat[56..60].copy_from_slice(&4096u32.to_le_bytes()); // st_blksize
        stat[64..72].copy_from_slice(&size.div_ceil(512).to_le_bytes());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        for at in [72, 88, 104] {
            stat[at..at + 8].copy_from_slice(&now.to_le_bytes());
        }
        self.write(address, &stat)?;
        Ok(0)
    }

    /// Run our version of `function`; its result goes in x0
    fn run(&mut self, function: Libc) -> Result<u64, String> {
        let a0 = self.argument(0)?;
        let a1 = self.argument(1)?;
        let a2 = self.argument(2)?;
        Ok(match function {
            Libc::Nothing => 0,
            Libc::Malloc => match self.heap.allocate(a0) {
                Some(address) => address,
                None => {
                    self.set_errno(ENOMEM);
                    0
                }
            },
            Libc::Free => {
                self.heap.release(a0);
                0
            }
            Libc::Errno => ERRNO,
            Libc::Umask => 0o022,
            Libc::Arc4random => {
                let mut bytes = [0u8; 4];
                getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
                u32::from_le_bytes(bytes) as u64
            }
            Libc::Gettimeofday => {
                if a0 != 0 {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default();
                    let mut tv = [0u8; 16];
                    tv[..8].copy_from_slice(&now.as_secs().to_le_bytes());
                    tv[8..].copy_from_slice(&(now.subsec_micros() as u64).to_le_bytes());
                    self.write(a0, &tv)?;
                }
                0
            }
            Libc::SystemPropertyGet => {
                // As Provision answers, for any property
                let value = b"no s/n number\0";
                self.write(a1, value)?;
                (value.len() - 1) as u64
            }
            Libc::Strncpy => {
                let source = self.read_string(a1)?;
                let mut bytes = source.into_bytes();
                bytes.truncate(a2 as usize);
                bytes.resize(a2 as usize, 0);
                self.write(a0, &bytes)?;
                a0
            }
            Libc::Dlopen => {
                let path = self.read_string(a0)?;
                let name = path.rsplit('/').next().unwrap_or(&path);
                match self.libraries.iter().position(|l| l.name == name) {
                    Some(index) => HANDLES + 0x10 * index as u64,
                    None => 0,
                }
            }
            Libc::Dlsym => {
                let name = self.read_string(a1)?;
                let index = a0.wrapping_sub(HANDLES) / 0x10;
                self.libraries
                    .get(index as usize)
                    .and_then(|l| l.elf.export(&name).map(|value| l.base + value))
                    .unwrap_or(0)
            }
            Libc::Dlclose => 0,
            Libc::Open => {
                let path = normalise(&self.read_string(a0)?);
                let flags = a1;
                let directory = self.files.directories.contains(&path) || path == "/";
                if !directory {
                    let exists = self.files.files.contains_key(&path);
                    if exists && flags & O_CREAT != 0 && flags & O_EXCL != 0 {
                        return Ok(self.fail(EEXIST));
                    }
                    if !exists {
                        if flags & O_CREAT == 0 || !self.files.parent_exists(&path) {
                            return Ok(self.fail(ENOENT));
                        }
                        self.files.files.insert(path.clone(), Vec::new());
                    } else if flags & O_TRUNC != 0 && flags & O_ACCMODE != 0 {
                        self.files.files.insert(path.clone(), Vec::new());
                    }
                }
                let fd = self.next_fd;
                self.next_fd += 1;
                self.open_files.insert(
                    fd,
                    OpenFile {
                        path,
                        position: 0,
                        directory,
                        append: flags & O_APPEND != 0,
                    },
                );
                fd as u64
            }
            Libc::Close => match self.open_files.remove(&(a0 as i32)) {
                Some(_) => 0,
                None => self.fail(EBADF),
            },
            Libc::Read => {
                let Some(open) = self.open_files.get(&(a0 as i32)) else {
                    return Ok(self.fail(EBADF));
                };
                if open.directory {
                    return Ok(self.fail(EISDIR));
                }
                let content = self
                    .files
                    .files
                    .get(&open.path)
                    .cloned()
                    .unwrap_or_default();
                let from = open.position.min(content.len());
                let to = (from + a2 as usize).min(content.len());
                self.write(a1, &content[from..to])?;
                if let Some(open) = self.open_files.get_mut(&(a0 as i32)) {
                    open.position = to;
                }
                (to - from) as u64
            }
            Libc::Write => {
                let bytes = self.read(a1, a2 as usize)?;
                let Some(open) = self.open_files.get_mut(&(a0 as i32)) else {
                    return Ok(self.fail(EBADF));
                };
                if open.directory {
                    return Ok(self.fail(EISDIR));
                }
                let content = self.files.files.entry(open.path.clone()).or_default();
                if open.append {
                    open.position = content.len();
                }
                let end = open.position + bytes.len();
                if content.len() < end {
                    content.resize(end, 0);
                }
                content[open.position..end].copy_from_slice(&bytes);
                open.position = end;
                bytes.len() as u64
            }
            Libc::Ftruncate => {
                let Some(open) = self.open_files.get(&(a0 as i32)) else {
                    return Ok(self.fail(EBADF));
                };
                let content = self.files.files.entry(open.path.clone()).or_default();
                content.resize(a1 as usize, 0);
                0
            }
            Libc::Fstat => {
                let Some(open) = self.open_files.get(&(a0 as i32)) else {
                    return Ok(self.fail(EBADF));
                };
                let path = open.path.clone();
                self.write_stat(a1, &path)?
            }
            Libc::Lstat => {
                let path = normalise(&self.read_string(a0)?);
                self.write_stat(a1, &path)?
            }
            Libc::Mkdir => {
                let path = normalise(&self.read_string(a0)?);
                if self.files.directories.contains(&path) || self.files.files.contains_key(&path) {
                    self.fail(EEXIST)
                } else if !self.files.parent_exists(&path) {
                    self.fail(ENOENT)
                } else {
                    self.files.directories.insert(path);
                    0
                }
            }
            Libc::Chmod => {
                let path = normalise(&self.read_string(a0)?);
                if self.files.directories.contains(&path) || self.files.files.contains_key(&path) {
                    0
                } else {
                    self.fail(ENOENT)
                }
            }
        })
    }

    fn stop(&mut self, failure: String) {
        self.failure.get_or_insert(failure);
        // SAFETY: stopping our own engine from inside a hook is allowed
        unsafe { (self.unicorn.emu_stop)(self.engine) };
    }
}

unsafe extern "C" fn on_import(_engine: Engine, address: u64, _size: u32, user: *mut c_void) {
    // SAFETY: `user` is the machine's Inner (see `set_up`), not otherwise
    // in use while emulation runs
    let inner = unsafe { &mut *user.cast::<Inner>() };
    let index = ((address - IMPORTS) / 4) as usize;
    let result = match inner.slots.get(index) {
        Some(Slot::Libc(function)) => {
            let function = *function;
            inner.last_import = Some(format!("{function:?}"));
            inner.run(function)
        }
        Some(Slot::Missing(name)) => Err(format!(
            "the library called {name}, which the emulated libc doesn't provide"
        )),
        None => Err(format!("a jump to an unused import slot at {address:#x}")),
    };
    match result.and_then(|value| inner.set_register(unicorn::REG_X0, value)) {
        Ok(()) => {}
        Err(failure) => inner.stop(failure),
    }
}

unsafe extern "C" fn on_unmapped(
    _engine: Engine,
    kind: c_int,
    address: u64,
    _size: c_int,
    _value: i64,
    user: *mut c_void,
) -> bool {
    // SAFETY: as in `on_import`
    let inner = unsafe { &mut *user.cast::<Inner>() };
    let pc = inner.register(unicorn::REG_PC).unwrap_or(0);
    let what = match kind {
        // uc_mem_type
        19 => "read",
        20 => "write",
        21 => "run",
        _ => "access",
    };
    inner.failure.get_or_insert(format!(
        "the library tried to {what} unmapped memory at {address:#x} (pc {pc:#x})"
    ));
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_paths() {
        assert_eq!(normalise("/a//b/./c/../d/"), "/a/b/d");
        assert_eq!(normalise("adi"), "/adi");
        assert_eq!(normalise("/"), "/");
    }

    #[test]
    fn creates_directories_above() {
        let mut files = Files::default();
        files.create_directories("/data/adi");
        assert!(files.directories.contains("/data"));
        assert!(files.directories.contains("/data/adi"));
        assert!(files.parent_exists("/data/adi/adi.pb"));
        assert!(!files.parent_exists("/other/x"));
    }

    #[test]
    fn heap_reuses_freed_blocks() {
        let mut heap = Heap {
            next: HEAP,
            sizes: HashMap::new(),
            free: HashMap::new(),
        };
        let a = heap.allocate(10).unwrap();
        let b = heap.allocate(20).unwrap();
        assert_eq!(b - a, 16);
        heap.release(a);
        assert_eq!(heap.allocate(16), Some(a));
        assert!(heap.allocate(HEAP_SIZE).is_none());
    }
}

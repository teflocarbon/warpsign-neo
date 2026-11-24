//! Unicorn, loaded at run time.
//!
//! Unicorn is GPLv2-only, so warpsign-neo never links or ships it: the app
//! downloads its own interpreter build (the one the App Store tab uses) and
//! the CLI takes a path to one. Only the handful of functions used here are
//! looked up, with the signatures unicorn.h (2.1) gives them.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::path::Path;

pub type Engine = *mut c_void;
pub type HookHandle = usize;

pub const UC_ARCH_ARM64: c_int = 2;
pub const UC_MODE_ARM: c_int = 0;
pub const UC_PROT_ALL: u32 = 7;
pub const UC_HOOK_CODE: c_int = 1 << 2;
/// Read, write and fetch of unmapped memory
pub const UC_HOOK_MEM_UNMAPPED: c_int = (1 << 4) | (1 << 5) | (1 << 6);

// uc_arm64_reg (arm64.h)
pub const REG_X29: c_int = 1;
pub const REG_X30: c_int = 2;
pub const REG_SP: c_int = 4;
pub const REG_X0: c_int = 199;
pub const REG_PC: c_int = 260;
pub const REG_CPACR_EL1: c_int = 261;
pub const REG_TPIDR_EL0: c_int = 263;

/// `void (*)(uc_engine *, uint64_t address, uint32_t size, void *user)`
pub type CodeHook = unsafe extern "C" fn(Engine, u64, u32, *mut c_void);
/// `bool (*)(uc_engine *, uc_mem_type, uint64_t address, int size, int64_t value, void *user)`
pub type MemHook = unsafe extern "C" fn(Engine, c_int, u64, c_int, i64, *mut c_void) -> bool;

/// The functions used, looked up once
pub struct Unicorn {
    pub open: unsafe extern "C" fn(c_int, c_int, *mut Engine) -> c_int,
    pub close: unsafe extern "C" fn(Engine) -> c_int,
    pub mem_map: unsafe extern "C" fn(Engine, u64, usize, u32) -> c_int,
    pub mem_write: unsafe extern "C" fn(Engine, u64, *const c_void, usize) -> c_int,
    pub mem_read: unsafe extern "C" fn(Engine, u64, *mut c_void, usize) -> c_int,
    pub reg_write: unsafe extern "C" fn(Engine, c_int, *const c_void) -> c_int,
    pub reg_read: unsafe extern "C" fn(Engine, c_int, *mut c_void) -> c_int,
    pub emu_start: unsafe extern "C" fn(Engine, u64, u64, u64, usize) -> c_int,
    pub emu_stop: unsafe extern "C" fn(Engine) -> c_int,
    /// Variadic in C; nothing is passed after `end` for these hook types
    pub hook_add: unsafe extern "C" fn(
        Engine,
        *mut HookHandle,
        c_int,
        *mut c_void,
        *mut c_void,
        u64,
        u64,
        ...
    ) -> c_int,
    pub strerror: unsafe extern "C" fn(c_int) -> *const c_char,
    // Keeps the library loaded for as long as the pointers above are used
    _library: Option<libloading::Library>,
}

// SAFETY: the function pointers are plain C functions; engines themselves
// aren't shared between threads (see Machine).
unsafe impl Send for Unicorn {}
unsafe impl Sync for Unicorn {}

impl Unicorn {
    /// Load Unicorn from a library file
    pub fn load(path: &Path) -> Result<Self, String> {
        // SAFETY: loading a library runs its initialisers; the caller
        // chose this file as Unicorn
        let library = unsafe { libloading::Library::new(path) }
            .map_err(|e| format!("couldn't load Unicorn from {}: {e}", path.display()))?;
        Self::from_library(library)
    }

    /// Use the Unicorn this process already loaded globally (the iOS app
    /// loads it with RTLD_GLOBAL for ipatool)
    #[cfg(unix)]
    pub fn already_loaded() -> Result<Self, String> {
        let library: libloading::Library = libloading::os::unix::Library::this().into();
        Self::from_library(library)
    }

    fn from_library(library: libloading::Library) -> Result<Self, String> {
        fn function<T: Copy>(library: &libloading::Library, name: &str) -> Result<T, String> {
            // SAFETY: each caller asks for the type unicorn.h gives the
            // function of that name (the field it's stored in)
            let symbol = unsafe { library.get::<T>(name.as_bytes()) }
                .map_err(|_| format!("this Unicorn has no {name}"))?;
            Ok(*symbol)
        }
        macro_rules! get {
            ($name:literal) => {
                function(&library, $name)?
            };
        }
        Ok(Self {
            open: get!("uc_open"),
            close: get!("uc_close"),
            mem_map: get!("uc_mem_map"),
            mem_write: get!("uc_mem_write"),
            mem_read: get!("uc_mem_read"),
            reg_write: get!("uc_reg_write"),
            reg_read: get!("uc_reg_read"),
            emu_start: get!("uc_emu_start"),
            emu_stop: get!("uc_emu_stop"),
            hook_add: get!("uc_hook_add"),
            strerror: get!("uc_strerror"),
            _library: Some(library),
        })
    }

    pub fn describe(&self, err: c_int) -> String {
        // SAFETY: uc_strerror returns a static string for any code
        let text = unsafe { (self.strerror)(err) };
        if text.is_null() {
            return format!("Unicorn error {err}");
        }
        // SAFETY: as above
        let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
        format!("{text} (Unicorn error {err})")
    }
}

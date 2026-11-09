//! Unicorn, the emulator ipatool runs Apple's code in, loaded at run time.
//!
//! Unicorn is GPLv2-only, so WarpSign doesn't ship it: the app downloads
//! a build of it (ipatool-bridge/unicorn), signs it with the user's own
//! certificate (`sign_library`) and loads it here, before ipatool needs it.
//! ipatool finds it with dlsym(RTLD_DEFAULT), so it's loaded globally.
//!
//! Unicorn compiles the code it runs, which needs JIT. On iOS 26 and later
//! (TXM), executable memory has to be prepared by a debugger (StikDebug's
//! universal script) and written through a second, writable mapping:
//! `set_up_jit` asks the debugger for one region, maps it again writable,
//! and gives both to Unicorn's split W^X mode (ipatool-bridge/unicorn/
//! splitwx.patch), which hands each engine a slice of it.

use std::path::Path;
use std::sync::Mutex;

use warpsign_core::signing::sign_macho_file;

use crate::{IdentityData, WarpsignError};

/// The executable region Unicorn's code buffers come from
const REGION_SIZE: usize = 512 << 20;
/// One engine's code buffer (ipatool uses 64 MB on Windows too); the
/// region holds 8, and ipatool keeps a few engines open at once
const SLICE_SIZE: usize = 64 << 20;

fn error(message: impl Into<String>) -> WarpsignError {
    WarpsignError::Store {
        message: message.into(),
    }
}

/// Sign a downloaded library (Unicorn) in place with `identity`, so iOS
/// lets the app load it: it must carry the app's own team ID.
#[uniffi::export]
pub fn sign_library(
    path: String,
    identity: IdentityData,
    team_id: String,
    identifier: String,
) -> Result<(), WarpsignError> {
    let signing = crate::account::signing_identity_of(&identity)?;
    sign_macho_file(Path::new(&path), &signing, &team_id, &identifier).map_err(|e| {
        WarpsignError::Signing {
            message: e.to_string(),
        }
    })
}

/// What has been done so far, in this process
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct EmulatorState {
    /// Unicorn is loaded
    pub loaded: bool,
    /// A debugger has attached at some point (CS_DEBUGGED): JIT is allowed
    pub debugged: bool,
    /// A debugger is attached now (P_TRACED). Enabling JIT ahead, from
    /// StikDebug, leaves the app debugged but with nothing attached, and
    /// iOS 26's region calls need the script there to answer them.
    pub attached: bool,
    /// Unicorn has its executable memory: ipatool can sign in and download
    pub ready: bool,
}

static STATE: Mutex<(bool, bool)> = Mutex::new((false, false));

#[uniffi::export]
pub fn emulator_state() -> EmulatorState {
    let (loaded, ready) = *STATE.lock().unwrap_or_else(|e| e.into_inner());
    EmulatorState {
        loaded,
        debugged: sys::debugged(),
        attached: sys::attached(),
        ready,
    }
}

/// Load the (signed) Unicorn library at `path`, once
#[uniffi::export]
pub fn load_unicorn(path: String) -> Result<(), WarpsignError> {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.0 {
        return Ok(());
    }
    sys::load(&path).map_err(error)?;
    state.0 = true;
    Ok(())
}

/// Give Unicorn its executable memory, once, after `load_unicorn`.
///
/// `through_debugger`: the region comes from the attached debugger's
/// script (iOS 26 and later), which is told to detach afterwards. Call it
/// only once `emulator_state().attached`, with StikDebug's universal script:
/// a breakpoint with nothing attached crashes the app. Otherwise (the
/// simulator, or JIT that needs no script) the region is mapped here.
#[uniffi::export]
pub fn set_up_jit(through_debugger: bool) -> Result<(), WarpsignError> {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.1 {
        return Ok(());
    }
    if !state.0 {
        return Err(error("Unicorn isn't loaded"));
    }
    // A breakpoint with no debugger to answer it kills the app
    if through_debugger && !sys::attached() {
        return Err(error("no debugger is attached to set up JIT"));
    }
    sys::set_up(through_debugger, REGION_SIZE, SLICE_SIZE).map_err(error)?;
    state.1 = true;
    Ok(())
}

/// Check Unicorn runs code, after `set_up_jit`: emulates a short arm64
/// loop (a thousand times, add 3) and returns its result, 3000
#[uniffi::export]
pub fn check_unicorn() -> Result<u64, WarpsignError> {
    if !emulator_state().ready {
        return Err(error("Unicorn isn't set up"));
    }
    sys::run_check().map_err(error)
}

#[cfg(target_vendor = "apple")]
mod sys {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};

    const RTLD_NOW: c_int = 0x2;
    const RTLD_GLOBAL: c_int = 0x8;
    const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;
    const PROT_READ: c_int = 0x1;
    const PROT_WRITE: c_int = 0x2;
    const PROT_EXEC: c_int = 0x4;
    const VM_FLAGS_ANYWHERE: c_int = 0x1;
    const VM_INHERIT_NONE: u32 = 2;
    const CS_OPS_STATUS: u32 = 0;
    const CS_DEBUGGED: u32 = 0x1000_0000;

    type Configure = unsafe extern "C" fn(*mut c_void, *mut c_void, usize, usize) -> c_int;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
        fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
        fn getpid() -> c_int;
        fn csops(pid: c_int, ops: u32, useraddr: *mut c_void, usersize: usize) -> c_int;
        fn sysctl(
            name: *mut c_int,
            namelen: u32,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> c_int;
        static mach_task_self_: u32;
        fn mach_vm_allocate(task: u32, address: *mut u64, size: u64, flags: c_int) -> c_int;
        fn mach_vm_deallocate(task: u32, address: u64, size: u64) -> c_int;
        #[allow(clippy::too_many_arguments)]
        fn mach_vm_remap(
            target_task: u32,
            target_address: *mut u64,
            size: u64,
            mask: u64,
            flags: c_int,
            src_task: u32,
            src_address: u64,
            copy: c_int,
            cur_protection: *mut c_int,
            max_protection: *mut c_int,
            inheritance: u32,
        ) -> c_int;
    }

    pub fn debugged() -> bool {
        let mut flags: u32 = 0;
        // SAFETY: CS_OPS_STATUS writes the process's code-signing flags
        // into the u32 given
        let status = unsafe {
            csops(
                getpid(),
                CS_OPS_STATUS,
                (&mut flags as *mut u32).cast(),
                std::mem::size_of::<u32>(),
            )
        };
        status == 0 && flags & CS_DEBUGGED != 0
    }

    pub fn attached() -> bool {
        const CTL_KERN: c_int = 1;
        const KERN_PROC: c_int = 14;
        const KERN_PROC_PID: c_int = 1;
        const P_TRACED: i32 = 0x800;
        // struct kinfo_proc (648 bytes on 64-bit Darwin) starts with
        // struct extern_proc, whose p_flag follows a 16-byte union and two
        // pointers
        const KINFO_PROC_SIZE: usize = 648;
        const P_FLAG_OFFSET: usize = 32;

        let mut mib = [CTL_KERN, KERN_PROC, KERN_PROC_PID, unsafe { getpid() }];
        let mut info = [0u8; KINFO_PROC_SIZE];
        let mut size = info.len();
        // SAFETY: sysctl fills at most `size` bytes of `info`
        let status = unsafe {
            sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                info.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if status != 0 || size < P_FLAG_OFFSET + 4 {
            return false;
        }
        let flag = &info[P_FLAG_OFFSET..P_FLAG_OFFSET + 4];
        i32::from_ne_bytes([flag[0], flag[1], flag[2], flag[3]]) & P_TRACED != 0
    }

    fn last_dl_error() -> String {
        // SAFETY: dlerror returns NULL or a C string valid until the next dl call
        let message = unsafe { dlerror() };
        if message.is_null() {
            "unknown error".into()
        } else {
            unsafe { CStr::from_ptr(message) }
                .to_string_lossy()
                .into_owned()
        }
    }

    pub fn load(path: &str) -> Result<(), String> {
        let path = CString::new(path).map_err(|_| "bad library path".to_string())?;
        // SAFETY: loading a library the app downloaded and checked
        let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
        if handle.is_null() {
            return Err(format!("couldn't load Unicorn: {}", last_dl_error()));
        }
        if configure().is_none() {
            return Err("this Unicorn has no split W^X support (uc_splitwx_configure)".into());
        }
        Ok(())
    }

    fn configure() -> Option<Configure> {
        // SAFETY: looking up a NUL-terminated name in every loaded image
        let symbol = unsafe { dlsym(RTLD_DEFAULT, c"uc_splitwx_configure".as_ptr()) };
        // SAFETY: the symbol, when present, has this signature (unicorn.h)
        (!symbol.is_null())
            .then(|| unsafe { std::mem::transmute::<*mut c_void, Configure>(symbol) })
    }

    /// Map `rx` again at a new address: the two share their memory
    fn alias(rx: u64, size: usize) -> Result<u64, String> {
        let mut address = 0u64;
        let (mut cur, mut max) = (0, 0);
        // SAFETY: remapping our own memory to a new address of the kernel's choosing
        let kr = unsafe {
            mach_vm_remap(
                mach_task_self_,
                &mut address,
                size as u64,
                0,
                VM_FLAGS_ANYWHERE,
                mach_task_self_,
                rx,
                0,
                &mut cur,
                &mut max,
                VM_INHERIT_NONE,
            )
        };
        if kr != 0 {
            return Err(format!("couldn't map JIT memory twice (mach error {kr})"));
        }
        Ok(address)
    }

    fn protect(address: u64, size: usize, prot: c_int, what: &str) -> Result<(), String> {
        // SAFETY: changing the protection of memory this module mapped
        if unsafe { mprotect(address as *mut c_void, size, prot) } != 0 {
            return Err(format!(
                "couldn't make JIT memory {what}: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    pub fn set_up(through_debugger: bool, size: usize, slice: usize) -> Result<(), String> {
        let configure = configure().ok_or("Unicorn isn't loaded")?;
        let (rw, rx) = if through_debugger {
            // SAFETY: the caller checked a debugger is attached; its script
            // handles the breakpoint
            let rx = unsafe { debugger::prepare_region(std::ptr::null_mut(), size) } as u64;
            let mapped = if rx == 0 {
                Err(
                    "the debugger gave no JIT memory: was StikDebug's universal script used?"
                        .into(),
                )
            } else {
                alias(rx, size).and_then(|rw| {
                    protect(rw, size, PROT_READ | PROT_WRITE, "writable").map(|_| (rw, rx))
                })
            };
            // Always let the debugger go, or it waits forever
            // SAFETY: as above
            unsafe { debugger::detach() };
            mapped?
        } else {
            let mut rw = 0u64;
            // SAFETY: allocating fresh memory
            let kr = unsafe {
                mach_vm_allocate(mach_task_self_, &mut rw, size as u64, VM_FLAGS_ANYWHERE)
            };
            if kr != 0 {
                return Err(format!("couldn't allocate JIT memory (mach error {kr})"));
            }
            let rx = alias(rw, size)
                .and_then(|rx| protect(rx, size, PROT_READ | PROT_EXEC, "executable").map(|_| rx));
            match rx {
                Ok(rx) => (rw, rx),
                Err(e) => {
                    // SAFETY: freeing what was just allocated, unused
                    unsafe { mach_vm_deallocate(mach_task_self_, rw, size as u64) };
                    return Err(e);
                }
            }
        };
        // SAFETY: both mappings are `size` bytes of the same memory, a page
        // multiple apart, kept for the life of the process
        let err = unsafe { configure(rw as *mut c_void, rx as *mut c_void, size, slice) };
        if err != 0 {
            return Err(format!("Unicorn refused the JIT memory (error {err})"));
        }
        Ok(())
    }

    type Engine = *mut c_void;

    /// Looked up in Unicorn, with the signature unicorn.h gives it
    macro_rules! unicorn_fn {
        ($name:literal: $ty:ty) => {{
            // SAFETY: a NUL-terminated name, looked up in every loaded image
            let symbol = unsafe { dlsym(RTLD_DEFAULT, concat!($name, "\0").as_ptr().cast()) };
            if symbol.is_null() {
                return Err(concat!("Unicorn has no ", $name).to_string());
            }
            // SAFETY: Unicorn's function of that name has this signature
            unsafe { std::mem::transmute::<*mut c_void, $ty>(symbol) }
        }};
    }

    pub fn run_check() -> Result<u64, String> {
        const UC_ARCH_ARM64: c_int = 2;
        const UC_MODE_ARM: c_int = 0;
        const UC_PROT_ALL: u32 = 7;
        const UC_ARM64_REG_X0: c_int = 199;
        const BASE: u64 = 0x10000;
        // mov x0, #0; mov x1, #1000; loop: add x0, x0, #3;
        // subs x1, x1, #1; b.ne loop
        const CODE: [u32; 5] = [
            0xD280_0000,
            0xD280_7D01,
            0x9100_0C00,
            0xF100_0421,
            0x54FF_FFC1,
        ];

        let open = unicorn_fn!("uc_open": unsafe extern "C" fn(c_int, c_int, *mut Engine) -> c_int);
        let close = unicorn_fn!("uc_close": unsafe extern "C" fn(Engine) -> c_int);
        let map = unicorn_fn!("uc_mem_map": unsafe extern "C" fn(Engine, u64, usize, u32) -> c_int);
        let write = unicorn_fn!("uc_mem_write": unsafe extern "C" fn(Engine, u64, *const c_void, usize) -> c_int);
        let start = unicorn_fn!("uc_emu_start": unsafe extern "C" fn(Engine, u64, u64, u64, usize) -> c_int);
        let read =
            unicorn_fn!("uc_reg_read": unsafe extern "C" fn(Engine, c_int, *mut c_void) -> c_int);

        let mut engine: Engine = std::ptr::null_mut();
        // SAFETY: Unicorn's API, used as documented, on an engine closed below
        unsafe {
            let err = open(UC_ARCH_ARM64, UC_MODE_ARM, &mut engine);
            if err != 0 {
                return Err(format!("uc_open failed (error {err})"));
            }
            let code: Vec<u8> = CODE.iter().flat_map(|i| i.to_le_bytes()).collect();
            let mut x0 = 0u64;
            let mut err = map(engine, BASE, 0x1000, UC_PROT_ALL);
            if err == 0 {
                err = write(engine, BASE, code.as_ptr().cast(), code.len());
            }
            if err == 0 {
                err = start(engine, BASE, BASE + code.len() as u64, 0, 0);
            }
            if err == 0 {
                err = read(engine, UC_ARM64_REG_X0, (&mut x0 as *mut u64).cast());
            }
            close(engine);
            if err != 0 {
                return Err(format!("emulating failed (Unicorn error {err})"));
            }
            Ok(x0)
        }
    }

    /// StikDebug's universal JIT protocol for iOS 26 and later
    /// (StikJIT's INTEGRATION.md): the attached script handles `brk #0xf00d`
    /// by the command in x16, and moves past it.
    #[cfg(target_arch = "aarch64")]
    mod debugger {
        use std::arch::asm;
        use std::ffi::c_void;

        /// Command 1: prepare an executable region of `length` bytes at
        /// `address` (NULL: the debugger allocates it); its address, or NULL
        #[inline(never)]
        pub unsafe fn prepare_region(address: *mut c_void, length: usize) -> *mut c_void {
            let mut x0 = address as usize;
            // SAFETY: the caller guarantees the script is attached
            unsafe {
                asm!(
                    "mov x16, #1",
                    "brk #0xf00d",
                    inout("x0") x0,
                    in("x1") length,
                    out("x16") _,
                    options(nostack),
                );
            }
            x0 as *mut c_void
        }

        /// Command 0: done, detach
        #[inline(never)]
        pub unsafe fn detach() {
            // SAFETY: the caller guarantees the script is attached
            unsafe {
                asm!("mov x16, #0", "brk #0xf00d", out("x16") _, options(nostack));
            }
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    mod debugger {
        use std::ffi::c_void;

        pub unsafe fn prepare_region(_: *mut c_void, _: usize) -> *mut c_void {
            std::ptr::null_mut()
        }

        pub unsafe fn detach() {}
    }
}

#[cfg(not(target_vendor = "apple"))]
mod sys {
    pub fn debugged() -> bool {
        false
    }

    pub fn attached() -> bool {
        false
    }

    pub fn load(_: &str) -> Result<(), String> {
        Err("Unicorn is only loaded on Apple platforms".into())
    }

    pub fn set_up(_: bool, _: usize, _: usize) -> Result<(), String> {
        Err("Unicorn is only loaded on Apple platforms".into())
    }

    pub fn run_check() -> Result<u64, String> {
        Err("Unicorn is only loaded on Apple platforms".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jit_needs_unicorn_first() {
        assert!(!emulator_state().ready);
        assert!(set_up_jit(false).is_err());
    }
}

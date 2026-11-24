//! Apple's ADI, from the Android libraries, run in [`Machine`].
//!
//! `libstoreservicescore.so` exports ADI's functions under fixed obfuscated
//! names; which name is which, and their signatures, are as Provision
//! (Dadoum, LGPL) documents them.

use std::sync::Arc;

use super::machine::{Files, Machine};
use super::unicorn::Unicorn;

pub const STORE_SERVICES_CORE: &str = "libstoreservicescore.so";
pub const CORE_ADI: &str = "libCoreADI.so";

/// Where the libraries think they are, and where ADI keeps its state, in
/// the machine's in-memory file system
pub const LIBRARY_PATH: &str = "/lib";
pub const PROVISIONING_PATH: &str = "/adi";

/// The account ADI is used for: -2, as AuthKit uses for the device itself
pub const DSID: u64 = -2i64 as u64;

/// ADI's error code for "not provisioned"
pub const NOT_PROVISIONED: i32 = -45061;

const LOAD_LIBRARY_WITH_PATH: &str = "kq56gsgHG6";
const SET_ANDROID_ID: &str = "Sph98paBcz";
const SET_PROVISIONING_PATH: &str = "nf92ngaK92";
const PROVISIONING_DESTROY: &str = "fy34trz2st";
const PROVISIONING_END: &str = "uv5t6nhkui";
const PROVISIONING_START: &str = "rsegvyrt87";
const GET_LOGIN_CODE: &str = "aslgmuibau";
const DISPOSE: &str = "jk24uiwqrg";
const OTP_REQUEST: &str = "qi864985u0";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AdiError {
    /// ADI returned an error code
    #[error("ADI error {code}{}", describe(*code).map(|d| format!(" ({d})")).unwrap_or_default())]
    Code { code: i32 },
    /// The emulator or loader failed
    #[error("{0}")]
    Machine(String),
}

impl From<String> for AdiError {
    fn from(message: String) -> Self {
        AdiError::Machine(message)
    }
}

/// What some codes mean, as Provision lists them
fn describe(code: i32) -> Option<&'static str> {
    Some(match code {
        -45001 => "invalid parameters",
        -45003 => "invalid trust key",
        -45006 => "ptm and tk don't match the cpim sent",
        -45025 => "unknown session",
        -45036 => "time error",
        -45054 => "file error",
        -45061 => "not provisioned",
        -45063 => "provisioning already started",
        -45066 => "session already finished",
        -45075 => "library loading failed",
        _ => return None,
    })
}

fn result(value: u64) -> Result<(), AdiError> {
    match value as u32 as i32 {
        0 => Ok(()),
        code => Err(AdiError::Code { code }),
    }
}

/// The libraries, loaded and set up for one identifier
pub struct Adi {
    machine: Machine,
}

/// From `start_provisioning`: send `cpim` to Apple, then finish with
/// `session`
pub struct ProvisioningStart {
    pub cpim: Vec<u8>,
    pub session: u32,
}

/// From `request_otp`
pub struct OneTimePassword {
    pub otp: Vec<u8>,
    pub machine_id: Vec<u8>,
}

impl Adi {
    /// Load the two libraries (arm64-v8a, from Apple Music for Android)
    /// and set ADI up for `identifier`, with `files` as its saved state
    /// (empty the first time)
    pub fn new(
        unicorn: Arc<Unicorn>,
        store_services_core: &[u8],
        core_adi: &[u8],
        identifier: &str,
        mut files: Files,
    ) -> Result<Self, AdiError> {
        files.create_directories(LIBRARY_PATH);
        files.create_directories(PROVISIONING_PATH);
        let mut machine = Machine::new(unicorn, files)?;
        // CoreADI first: storeservicescore finds it with dlopen
        machine.load(CORE_ADI, core_adi)?;
        machine.load(STORE_SERVICES_CORE, store_services_core)?;
        let mut adi = Adi { machine };

        // In Provision's order: library, provisioning path, identifier
        let library = adi.machine.put(format!("{LIBRARY_PATH}\0").as_bytes())?;
        adi.call(LOAD_LIBRARY_WITH_PATH, &[library])?;
        let path = adi
            .machine
            .put(format!("{PROVISIONING_PATH}\0").as_bytes())?;
        adi.call(SET_PROVISIONING_PATH, &[path])?;
        let id = adi.machine.put(identifier.as_bytes())?;
        adi.call(SET_ANDROID_ID, &[id, identifier.len() as u64])?;
        for address in [path, library, id] {
            adi.machine.release(address);
        }
        Ok(adi)
    }

    fn function(&self, name: &str) -> Result<u64, AdiError> {
        self.machine
            .symbol(STORE_SERVICES_CORE, name)
            .ok_or_else(|| AdiError::Machine(format!("{STORE_SERVICES_CORE} has no {name}")))
    }

    fn call(&mut self, name: &str, arguments: &[u64]) -> Result<(), AdiError> {
        let address = self.function(name)?;
        result(self.machine.call(address, arguments)?)
    }

    /// Whether this identifier is provisioned with Apple
    pub fn is_provisioned(&mut self) -> Result<bool, AdiError> {
        match self.call(GET_LOGIN_CODE, &[DSID]) {
            Ok(()) => Ok(true),
            Err(AdiError::Code { code }) if code == NOT_PROVISIONED => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Step one, with `spim` from Apple's midStartProvisioning
    pub fn start_provisioning(&mut self, spim: &[u8]) -> Result<ProvisioningStart, AdiError> {
        let spim_at = self.machine.put(spim)?;
        // cpim pointer, its length, the session
        let out = self.machine.put(&[0u8; 24])?;
        let outcome = self.call(
            PROVISIONING_START,
            &[DSID, spim_at, spim.len() as u64, out, out + 8, out + 16],
        );
        let taken = outcome.and_then(|()| {
            let cpim_at = self.machine.read_u64(out)?;
            let length = self.machine.read_u32(out + 8)?;
            let session = self.machine.read_u32(out + 16)?;
            let cpim = self.machine.read(cpim_at, length as usize)?;
            self.call(DISPOSE, &[cpim_at])?;
            Ok(ProvisioningStart { cpim, session })
        });
        self.machine.release(spim_at);
        self.machine.release(out);
        taken
    }

    /// Step two, with `ptm` and `tk` from Apple's midFinishProvisioning
    pub fn end_provisioning(
        &mut self,
        session: u32,
        ptm: &[u8],
        tk: &[u8],
    ) -> Result<(), AdiError> {
        let ptm_at = self.machine.put(ptm)?;
        let tk_at = self.machine.put(tk)?;
        let outcome = self.call(
            PROVISIONING_END,
            &[
                session as u64,
                ptm_at,
                ptm.len() as u64,
                tk_at,
                tk.len() as u64,
            ],
        );
        self.machine.release(ptm_at);
        self.machine.release(tk_at);
        outcome
    }

    /// Give up on a session after a failed step
    pub fn destroy_provisioning(&mut self, session: u32) -> Result<(), AdiError> {
        self.call(PROVISIONING_DESTROY, &[session as u64])
    }

    /// The headers' one-time password and machine ID, fresh each time
    pub fn request_otp(&mut self) -> Result<OneTimePassword, AdiError> {
        // machine ID pointer and length, OTP pointer and length
        let out = self.machine.put(&[0u8; 32])?;
        let outcome = self.call(OTP_REQUEST, &[DSID, out, out + 8, out + 16, out + 24]);
        let taken = outcome.and_then(|()| {
            let mid_at = self.machine.read_u64(out)?;
            let mid_length = self.machine.read_u32(out + 8)?;
            let otp_at = self.machine.read_u64(out + 16)?;
            let otp_length = self.machine.read_u32(out + 24)?;
            let machine_id = self.machine.read(mid_at, mid_length as usize)?;
            let otp = self.machine.read(otp_at, otp_length as usize)?;
            self.call(DISPOSE, &[mid_at])?;
            self.call(DISPOSE, &[otp_at])?;
            Ok(OneTimePassword { otp, machine_id })
        });
        self.machine.release(out);
        taken
    }

    /// ADI's saved state (`adi.pb` and the like): keep it secret, and pass
    /// it back next time
    pub fn files(&self) -> &Files {
        self.machine.files()
    }

    /// Library imports the emulated libc doesn't provide (they only matter
    /// if called)
    pub fn missing_imports(&self) -> Vec<String> {
        self.machine.missing_imports()
    }
}

//! The App Store, through ipatool linked into the app (ipatool-bridge).
//!
//! The bridge is looked up when it's first used, not linked against: the
//! WarpSign app has it (and exports it, as ipatool needs for Unicorn), but
//! the Companion app, which links this crate too, doesn't, and needn't carry
//! it. Without it, every call fails with "not available".

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use warpsign_core::appstore::bridge::Bridge;
use warpsign_core::appstore::{AppRef, AppStore, DownloadRequest, StoreError};

use crate::WarpsignError;

type CallFn = unsafe extern "C" fn(*const c_char) -> *mut c_char;
type FreeFn = unsafe extern "C" fn(*mut c_char);

struct Symbols {
    call: CallFn,
    free: FreeFn,
}

fn symbols() -> Option<&'static Symbols> {
    static SYMBOLS: OnceLock<Option<Symbols>> = OnceLock::new();
    SYMBOLS.get_or_init(lookup).as_ref()
}

#[cfg(target_vendor = "apple")]
fn lookup() -> Option<Symbols> {
    use std::ffi::c_void;

    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    // Apple's RTLD_DEFAULT: every image loaded, the app itself included
    let default = -2isize as *mut c_void;
    // SAFETY: dlsym with NUL-terminated names; the symbols, when present,
    // are ipatool-bridge's exported functions with these signatures
    unsafe {
        let call = dlsym(default, c"warpsign_ipatool_call".as_ptr());
        let free = dlsym(default, c"warpsign_ipatool_free".as_ptr());
        if call.is_null() || free.is_null() {
            return None;
        }
        Some(Symbols {
            call: std::mem::transmute::<*mut c_void, CallFn>(call),
            free: std::mem::transmute::<*mut c_void, FreeFn>(free),
        })
    }
}

#[cfg(not(target_vendor = "apple"))]
fn lookup() -> Option<Symbols> {
    None
}

const UNAVAILABLE: &str =
    r#"{"level":"error","error":"the App Store isn't available in this build","success":false}"#;

fn call(request: &str) -> String {
    let Some(symbols) = symbols() else {
        return UNAVAILABLE.into();
    };
    let Ok(request) = CString::new(request) else {
        return r#"{"level":"error","error":"request contains a NUL","success":false}"#.into();
    };
    // SAFETY: the bridge takes a C string and returns one it allocated,
    // which goes back to it to free
    unsafe {
        let reply = (symbols.call)(request.as_ptr());
        if reply.is_null() {
            return r#"{"level":"error","error":"no reply from ipatool","success":false}"#.into();
        }
        let text = CStr::from_ptr(reply).to_string_lossy().into_owned();
        (symbols.free)(reply);
        text
    }
}

/// Whether this build has ipatool in it
#[uniffi::export]
pub fn app_store_available() -> bool {
    symbols().is_some()
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StoreAccountInfo {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StoreAppInfo {
    /// Numeric App Store ID
    pub id: i64,
    pub bundle_id: String,
    pub name: String,
    pub version: String,
    /// In the storefront's currency; 0 for free apps
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StoreDownloadInfo {
    pub path: String,
    /// Whether a licence was got for it on the way
    pub purchased: bool,
}

fn store_error(error: StoreError) -> WarpsignError {
    let message = error.to_string();
    match error {
        StoreError::NotSignedIn => WarpsignError::StoreSignedOut { message },
        StoreError::CodeRequired => WarpsignError::StoreCodeRequired { message },
        StoreError::LicenseRequired => WarpsignError::StoreNotOwned { message },
        _ => WarpsignError::Store { message },
    }
}

fn app_ref(app: &str) -> Result<AppRef, WarpsignError> {
    app.parse()
        .map_err(|message| WarpsignError::Store { message })
}

/// ipatool's App Store client. Calls block (downloads for minutes): call
/// them off the main thread. One call runs at a time.
#[derive(uniffi::Object)]
pub struct AppStoreClient {
    bridge: Bridge<fn(&str) -> String>,
}

#[uniffi::export]
impl AppStoreClient {
    /// `home`: where ipatool keeps its state (an ipatool folder is made in
    /// it). `passphrase`: for ipatool's keychain file; keep it in the
    /// Keychain. `mac_address`: made up once and kept, e.g.
    /// "02:1a:2b:3c:4d:5e", since iOS gives apps none.
    #[uniffi::constructor]
    pub fn new(
        home: String,
        passphrase: String,
        mac_address: String,
    ) -> Result<Arc<Self>, WarpsignError> {
        let bridge = Bridge::new(call as fn(&str) -> String);
        bridge
            .configure(&home, &passphrase, &mac_address)
            .map_err(store_error)?;
        Ok(Arc::new(Self { bridge }))
    }

    /// Load ipatool's runtime and fetch the Apple assets it needs. No
    /// account needed; sign-in does it otherwise.
    pub fn prepare(&self) -> Result<(), WarpsignError> {
        self.bridge.prepare().map_err(store_error)
    }

    /// Fails with StoreCodeRequired when Apple wants a two-factor code: ask
    /// for it, then sign in again with it
    pub fn login(
        &self,
        email: String,
        password: String,
        code: Option<String>,
    ) -> Result<StoreAccountInfo, WarpsignError> {
        let account = self
            .bridge
            .login(&email, &password, code.as_deref())
            .map_err(store_error)?;
        Ok(StoreAccountInfo {
            name: account.name,
            email: account.email,
        })
    }

    /// The signed-in account, or None
    pub fn account(&self) -> Result<Option<StoreAccountInfo>, WarpsignError> {
        match self.bridge.account() {
            Ok(account) => Ok(Some(StoreAccountInfo {
                name: account.name,
                email: account.email,
            })),
            Err(StoreError::NotSignedIn) => Ok(None),
            Err(error) => Err(store_error(error)),
        }
    }

    pub fn logout(&self) -> Result<(), WarpsignError> {
        self.bridge.logout().map_err(store_error)
    }

    pub fn search(&self, term: String, limit: u32) -> Result<Vec<StoreAppInfo>, WarpsignError> {
        let apps = self
            .bridge
            .search(&term, limit, None)
            .map_err(store_error)?;
        Ok(apps
            .into_iter()
            .map(|app| StoreAppInfo {
                id: app.id,
                bundle_id: app.bundle_id,
                name: app.name,
                version: app.version,
                price: app.price,
            })
            .collect())
    }

    /// External version IDs, oldest first. `app`: bundle ID or App Store ID.
    pub fn versions(&self, app: String) -> Result<Vec<String>, WarpsignError> {
        let versions = self
            .bridge
            .versions(&app_ref(&app)?, None)
            .map_err(store_error)?;
        Ok(versions.external_ids)
    }

    /// Download into `directory` (ipatool names the file). `version`: an
    /// external version ID, or None for the latest. `purchase`: get the app
    /// first if the account doesn't own it (free apps only).
    pub fn download(
        &self,
        app: String,
        version: Option<String>,
        directory: String,
        purchase: bool,
    ) -> Result<StoreDownloadInfo, WarpsignError> {
        let download = self
            .bridge
            .download(&DownloadRequest {
                app: app_ref(&app)?,
                version,
                output: Some(PathBuf::from(directory)),
                purchase,
                platform: None,
            })
            .map_err(store_error)?;
        Ok(StoreDownloadInfo {
            path: download.path.display().to_string(),
            purchased: download.purchased,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_without_the_bridge() {
        // Tests don't link ipatool-bridge
        assert!(!app_store_available());
        match AppStoreClient::new("/tmp".into(), "x".into(), "02:00:00:00:00:01".into()) {
            Err(WarpsignError::Store { message }) => assert!(message.contains("isn't available")),
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("configured without a bridge"),
        }
    }
}

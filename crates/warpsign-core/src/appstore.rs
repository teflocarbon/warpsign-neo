//! The App Store: searching it, and downloading apps the Apple ID owns.
//!
//! [ipatool](https://github.com/majd/ipatool) (MIT) does the talking to
//! Apple. The store's protocol changes often, and ipatool now runs parts of
//! Apple's own store code in an emulator to keep up, so warpsign uses it
//! rather than port it: updating means a newer ipatool, not new code here.
//!
//! [`AppStore`] is what front-ends use. The CLI implements it by running the
//! `ipatool` program and reading the JSON it prints, which [`ipatool`]
//! parses. A build of ipatool linked into the iOS app can implement the same
//! trait.
//!
//! Downloads are still FairPlay-encrypted, as the App Store serves them.

use std::path::PathBuf;

/// Which of an app's platforms to search or download
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    IPhone,
    IPad,
    AppleTv,
    WatchOs,
    VisionOs,
    MacOs,
}

impl Platform {
    /// The name ipatool's `--platform` takes
    pub fn ipatool_name(self) -> &'static str {
        match self {
            Platform::IPhone => "iphone",
            Platform::IPad => "ipad",
            Platform::AppleTv => "appletv",
            Platform::WatchOs => "watchos",
            Platform::VisionOs => "visionos",
            Platform::MacOs => "macos",
        }
    }
}

/// An app, by bundle ID or by its numeric App Store ID (the `id…` in its
/// store link). The numeric ID still works for apps taken off the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppRef {
    BundleId(String),
    Id(i64),
}

impl std::str::FromStr for AppRef {
    type Err = String;

    /// All digits (optionally after `id`, as in store links) is an App Store
    /// ID, anything else a bundle ID
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err("no app given".into());
        }
        let digits = s.strip_prefix("id").unwrap_or(s);
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return digits
                .parse()
                .map(AppRef::Id)
                .map_err(|_| format!("not an App Store ID: {s}"));
        }
        Ok(AppRef::BundleId(s.to_string()))
    }
}

impl std::fmt::Display for AppRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppRef::BundleId(id) => f.write_str(id),
            AppRef::Id(id) => write!(f, "id{id}"),
        }
    }
}

/// The App Store account ipatool is signed in to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreAccount {
    pub name: String,
    pub email: String,
}

/// A search result
#[derive(Debug, Clone, PartialEq)]
pub struct StoreApp {
    /// Numeric App Store ID
    pub id: i64,
    pub bundle_id: String,
    pub name: String,
    /// Latest version on the store
    pub version: String,
    /// In the storefront's currency; 0 for free apps
    pub price: f64,
}

/// The versions the store still has for an app
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    pub bundle_id: Option<String>,
    /// External version IDs, as the store lists them (oldest first). These
    /// are what [`DownloadRequest::version`] takes, not version strings.
    pub external_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadRequest {
    pub app: AppRef,
    /// External version ID; the latest version when `None`
    pub version: Option<String>,
    /// File or directory to save to; ipatool names the file when `None`
    /// or a directory
    pub output: Option<PathBuf>,
    /// Get a licence first if the account doesn't have one (free apps only)
    pub purchase: bool,
    pub platform: Option<Platform>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub path: PathBuf,
    /// Whether a licence was got for it on the way
    pub purchased: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not signed in to the App Store")]
    NotSignedIn,
    #[error("the account doesn't own this app")]
    LicenseRequired,
    /// ipatool keeps its session in a passphrase-protected file where there's
    /// no system keychain (Windows, Linux without a secret service)
    #[error("ipatool's keychain needs its passphrase")]
    KeychainLocked,
    /// Signing in needs the two-factor code Apple has just sent
    #[error("Apple needs the two-factor code")]
    CodeRequired,
    #[error("{0}")]
    Failed(String),
    #[error("couldn't understand ipatool's output: {0}")]
    Output(String),
}

pub trait AppStore {
    fn account(&self) -> Result<StoreAccount, StoreError>;
    fn search(
        &self,
        term: &str,
        limit: u32,
        platform: Option<Platform>,
    ) -> Result<Vec<StoreApp>, StoreError>;
    fn versions(&self, app: &AppRef, platform: Option<Platform>) -> Result<Versions, StoreError>;
    fn download(&self, request: &DownloadRequest) -> Result<Download, StoreError>;
}

/// ipatool's command lines and `--format json` output.
///
/// Each line ipatool prints is one JSON object (it logs with zerolog). A
/// failure is a line with `"level":"error"` and the message in `error`;
/// results are the fields of an `info` line.
pub mod ipatool {
    use serde_json::{Map, Value};

    use super::*;

    /// Arguments for `ipatool search`, after the global flags
    pub fn search_args(term: &str, limit: u32, platform: Option<Platform>) -> Vec<String> {
        let mut args = vec![
            "search".into(),
            term.into(),
            "--limit".into(),
            limit.to_string(),
        ];
        push_platform(&mut args, platform);
        args
    }

    pub fn versions_args(app: &AppRef, platform: Option<Platform>) -> Vec<String> {
        let mut args = vec!["list-versions".into()];
        push_app(&mut args, app);
        push_platform(&mut args, platform);
        args
    }

    pub fn download_args(request: &DownloadRequest) -> Vec<String> {
        let mut args = vec!["download".into()];
        push_app(&mut args, &request.app);
        if let Some(version) = &request.version {
            args.extend(["--external-version-id".into(), version.clone()]);
        }
        if let Some(output) = &request.output {
            args.extend(["--output".into(), output.display().to_string()]);
        }
        if request.purchase {
            args.push("--purchase".into());
        }
        push_platform(&mut args, request.platform);
        args
    }

    fn push_app(args: &mut Vec<String>, app: &AppRef) {
        match app {
            AppRef::BundleId(id) => args.extend(["--bundle-identifier".into(), id.clone()]),
            AppRef::Id(id) => args.extend(["--app-id".into(), id.to_string()]),
        }
    }

    fn push_platform(args: &mut Vec<String>, platform: Option<Platform>) {
        if let Some(platform) = platform {
            args.extend(["--platform".into(), platform.ipatool_name().into()]);
        }
    }

    /// The fields of the last result line carrying `key`, or the error
    /// ipatool reported
    pub fn result(stdout: &str, key: &str) -> Result<Map<String, Value>, StoreError> {
        let lines: Vec<Map<String, Value>> = stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
            .filter_map(|value| match value {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .collect();
        if let Some(error) = lines
            .iter()
            .rev()
            .find(|line| line.get("level").and_then(Value::as_str) == Some("error"))
        {
            let message = error
                .get("error")
                .or_else(|| error.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("ipatool failed without saying why");
            return Err(classify(message));
        }
        lines
            .into_iter()
            .rev()
            .find(|line| line.contains_key(key))
            .ok_or_else(|| StoreError::Output(format!("no `{key}` in what ipatool printed")))
    }

    /// Recognise the failures front-ends act on. Messages are ipatool's
    /// (pkg/appstore and cmd), matched loosely so a reworded prefix still
    /// works.
    pub fn classify(message: &str) -> StoreError {
        let lower = message.to_ascii_lowercase();
        // First: ipatool wraps it in "failed to get account" when the
        // keychain file is locked, signed in or not
        if lower.contains("keychain passphrase is required") {
            StoreError::KeychainLocked
        } else if lower.contains("failed to get account") {
            StoreError::NotSignedIn
        } else if lower.contains("license is required") {
            StoreError::LicenseRequired
        } else if lower.contains("auth code is required") {
            StoreError::CodeRequired
        } else {
            StoreError::Failed(message.to_string())
        }
    }

    pub fn parse_account(stdout: &str) -> Result<StoreAccount, StoreError> {
        let line = result(stdout, "email")?;
        Ok(StoreAccount {
            name: string(&line, "name"),
            email: string(&line, "email"),
        })
    }

    pub fn parse_search(stdout: &str) -> Result<Vec<StoreApp>, StoreError> {
        let line = result(stdout, "apps")?;
        let apps = line
            .get("apps")
            .and_then(Value::as_array)
            .ok_or_else(|| StoreError::Output("`apps` isn't a list".into()))?;
        Ok(apps
            .iter()
            .filter_map(Value::as_object)
            .map(|app| StoreApp {
                id: app.get("id").and_then(Value::as_i64).unwrap_or_default(),
                bundle_id: string(app, "bundleID"),
                name: string(app, "name"),
                version: string(app, "version"),
                price: app.get("price").and_then(Value::as_f64).unwrap_or_default(),
            })
            .collect())
    }

    pub fn parse_versions(stdout: &str) -> Result<Versions, StoreError> {
        let line = result(stdout, "externalVersionIdentifiers")?;
        let external_ids = match line.get("externalVersionIdentifiers") {
            Some(Value::Array(ids)) => ids
                .iter()
                .map(|id| match id {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
                .collect(),
            // zerolog writes a nil slice as null
            Some(Value::Null) | None => Vec::new(),
            Some(_) => {
                return Err(StoreError::Output(
                    "`externalVersionIdentifiers` isn't a list".into(),
                ));
            }
        };
        let bundle_id = Some(string(&line, "bundleID")).filter(|id| !id.is_empty());
        Ok(Versions {
            bundle_id,
            external_ids,
        })
    }

    pub fn parse_download(stdout: &str) -> Result<Download, StoreError> {
        let line = result(stdout, "output")?;
        let path = string(&line, "output");
        if path.is_empty() {
            return Err(StoreError::Output("ipatool gave no output path".into()));
        }
        Ok(Download {
            path: path.into(),
            purchased: line
                .get("purchased")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    fn string(map: &Map<String, Value>, key: &str) -> String {
        map.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }
}

/// ipatool linked into the iOS app, through ipatool-bridge
/// (ipatool-bridge/bridge.go): one call taking a JSON request and returning
/// one JSON object, shaped as ipatool's `--format json` prints it, so
/// [`ipatool`] parses the replies.
pub mod bridge {
    use serde_json::{Value, json};

    use super::*;

    /// The bridge, through whatever calls it (`call` takes a request and
    /// returns the reply)
    pub struct Bridge<F: Fn(&str) -> String> {
        call: F,
    }

    impl<F: Fn(&str) -> String> Bridge<F> {
        pub fn new(call: F) -> Self {
            Self { call }
        }

        fn send(&self, request: Value) -> String {
            (self.call)(&request.to_string())
        }

        /// Where ipatool keeps its state (under `home`/ipatool), the
        /// passphrase for its keychain file, and the made-up MAC address
        /// Apple knows this device by. Before anything else.
        pub fn configure(
            &self,
            home: &str,
            passphrase: &str,
            mac_address: &str,
        ) -> Result<(), StoreError> {
            let reply = self.send(json!({
                "op": "configure",
                "home": home,
                "passphrase": passphrase,
                "mac_address": mac_address,
            }));
            ipatool::result(&reply, "success").map(|_| ())
        }

        /// Load ipatool's runtime and cache Apple's assets. Needs no account;
        /// otherwise done on the first sign-in.
        pub fn prepare(&self) -> Result<(), StoreError> {
            let reply = self.send(json!({"op": "prepare"}));
            ipatool::result(&reply, "success").map(|_| ())
        }

        /// [`StoreError::CodeRequired`] when Apple wants a two-factor code:
        /// sign in again with it
        pub fn login(
            &self,
            email: &str,
            password: &str,
            code: Option<&str>,
        ) -> Result<StoreAccount, StoreError> {
            let reply = self.send(json!({
                "op": "login",
                "email": email,
                "password": password,
                "auth_code": code.unwrap_or_default(),
            }));
            ipatool::parse_account(&reply)
        }

        pub fn logout(&self) -> Result<(), StoreError> {
            let reply = self.send(json!({"op": "logout"}));
            ipatool::result(&reply, "success").map(|_| ())
        }
    }

    fn platform_name(platform: Option<Platform>) -> &'static str {
        platform.map(Platform::ipatool_name).unwrap_or_default()
    }

    fn app_fields(app: &AppRef) -> (String, i64) {
        match app {
            AppRef::BundleId(id) => (id.clone(), 0),
            AppRef::Id(id) => (String::new(), *id),
        }
    }

    impl<F: Fn(&str) -> String> AppStore for Bridge<F> {
        fn account(&self) -> Result<StoreAccount, StoreError> {
            ipatool::parse_account(&self.send(json!({"op": "account"})))
        }

        fn search(
            &self,
            term: &str,
            limit: u32,
            platform: Option<Platform>,
        ) -> Result<Vec<StoreApp>, StoreError> {
            let reply = self.send(json!({
                "op": "search",
                "term": term,
                "limit": limit,
                "platform": platform_name(platform),
            }));
            ipatool::parse_search(&reply)
        }

        fn versions(
            &self,
            app: &AppRef,
            platform: Option<Platform>,
        ) -> Result<Versions, StoreError> {
            let (bundle_id, app_id) = app_fields(app);
            let reply = self.send(json!({
                "op": "versions",
                "bundle_id": bundle_id,
                "app_id": app_id,
                "platform": platform_name(platform),
            }));
            ipatool::parse_versions(&reply)
        }

        fn download(&self, request: &DownloadRequest) -> Result<Download, StoreError> {
            let (bundle_id, app_id) = app_fields(&request.app);
            let reply = self.send(json!({
                "op": "download",
                "bundle_id": bundle_id,
                "app_id": app_id,
                "version": request.version.clone().unwrap_or_default(),
                "output": request
                    .output
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                "purchase": request.purchase,
                "platform": platform_name(request.platform),
            }));
            ipatool::parse_download(&reply)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ipatool::*;
    use super::*;

    #[test]
    fn bridge_requests_and_replies() {
        use std::cell::RefCell;

        let sent = RefCell::new(Vec::<serde_json::Value>::new());
        let bridge = bridge::Bridge::new(|request: &str| {
            let request: serde_json::Value = serde_json::from_str(request).unwrap();
            let reply = match request["op"].as_str().unwrap() {
                "login" if request["auth_code"] == "" => {
                    r#"{"level":"error","error":"auth code is required","success":false}"#
                }
                "login" => {
                    r#"{"level":"info","name":"Test User","email":"test@example.com","success":true}"#
                }
                "download" => {
                    r#"{"level":"info","output":"/tmp/a.ipa","purchased":false,"success":true}"#
                }
                _ => r#"{"level":"info","success":true}"#,
            };
            sent.borrow_mut().push(request);
            reply.to_string()
        });

        bridge
            .configure("/home", "secret", "02:00:00:00:00:01")
            .unwrap();
        assert!(matches!(
            bridge.login("test@example.com", "pw", None),
            Err(StoreError::CodeRequired)
        ));
        let account = bridge
            .login("test@example.com", "pw", Some("123456"))
            .unwrap();
        assert_eq!(account.name, "Test User");
        let download = bridge
            .download(&DownloadRequest {
                app: AppRef::Id(111),
                version: None,
                output: Some(PathBuf::from("/tmp")),
                purchase: true,
                platform: Some(Platform::IPad),
            })
            .unwrap();
        assert_eq!(download.path, PathBuf::from("/tmp/a.ipa"));

        let sent = sent.borrow();
        assert_eq!(sent[0]["op"], "configure");
        assert_eq!(sent[0]["mac_address"], "02:00:00:00:00:01");
        assert_eq!(sent[2]["auth_code"], "123456");
        let download = &sent[3];
        assert_eq!(download["app_id"], 111);
        assert_eq!(download["bundle_id"], "");
        assert_eq!(download["output"], "/tmp");
        assert_eq!(download["purchase"], true);
        assert_eq!(download["platform"], "ipad");
    }

    // Shaped like ipatool 2.8.0's output (cmd/*.go), with made-up values

    #[test]
    fn app_refs() {
        assert_eq!(
            "com.example.app".parse(),
            Ok(AppRef::BundleId("com.example.app".into()))
        );
        assert_eq!("123456789".parse(), Ok(AppRef::Id(123456789)));
        assert_eq!("id123456789".parse(), Ok(AppRef::Id(123456789)));
        assert_eq!("idea.app".parse(), Ok(AppRef::BundleId("idea.app".into())));
        assert!("".parse::<AppRef>().is_err());
    }

    #[test]
    fn search() {
        let out = r#"{"level":"info","count":2,"apps":[{"id":111,"bundleID":"com.example.one","name":"One","version":"1.2","price":0},{"id":222,"bundleID":"com.example.two","name":"Two","version":"3.0","price":1.99}],"time":"2026-11-09T12:00:00Z"}"#;
        let apps = parse_search(out).unwrap();
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].bundle_id, "com.example.one");
        assert_eq!(apps[1].id, 222);
        assert_eq!(apps[1].price, 1.99);
    }

    #[test]
    fn empty_search() {
        let out = r#"{"level":"info","count":0,"apps":[],"time":"2026-11-09T12:00:00Z"}"#;
        assert!(parse_search(out).unwrap().is_empty());
    }

    #[test]
    fn versions() {
        let out = r#"{"level":"info","externalVersionIdentifiers":["800000001","800000002"],"bundleID":"com.example.one","success":true,"time":"2026-11-09T12:00:00Z"}"#;
        let versions = parse_versions(out).unwrap();
        assert_eq!(versions.bundle_id.as_deref(), Some("com.example.one"));
        assert_eq!(versions.external_ids, ["800000001", "800000002"]);
    }

    #[test]
    fn download_after_other_lines() {
        let out = "{\"level\":\"info\",\"message\":\"something first\"}\n\
            not json\n\
            {\"level\":\"info\",\"output\":\"/tmp/com.example.one_111_1.2.ipa\",\"purchased\":true,\"success\":true}\n";
        let download = parse_download(out).unwrap();
        assert_eq!(
            download.path,
            PathBuf::from("/tmp/com.example.one_111_1.2.ipa")
        );
        assert!(download.purchased);
    }

    #[test]
    fn account() {
        let out =
            r#"{"level":"info","name":"Test User","email":"test@example.com","success":true}"#;
        let account = parse_account(out).unwrap();
        assert_eq!(account.name, "Test User");
        assert_eq!(account.email, "test@example.com");
    }

    #[test]
    fn errors() {
        let line = |message: &str| {
            serde_json::json!({"level": "error", "error": message, "success": false}).to_string()
        };
        assert!(matches!(
            parse_account(&line(
                "failed to get account: failed to get item: The specified item could not be found in the keyring"
            )),
            Err(StoreError::NotSignedIn)
        ));
        assert!(matches!(
            parse_download(&line("license is required")),
            Err(StoreError::LicenseRequired)
        ));
        assert!(matches!(
            parse_search(&line(
                "keychain passphrase is required when not running in interactive mode; use the \"--keychain-passphrase\" flag"
            )),
            Err(StoreError::KeychainLocked)
        ));
        // As ipatool 2.8.0 on Windows says it, signed in with a locked file
        assert!(matches!(
            parse_account(&line(
                "failed to get account: failed to get item: keychain passphrase is required when not running in interactive mode; use the \"--keychain-passphrase\" flag"
            )),
            Err(StoreError::KeychainLocked)
        ));
        match parse_search(&line("received error: something else")) {
            Err(StoreError::Failed(message)) => {
                assert_eq!(message, "received error: something else")
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse_search(""), Err(StoreError::Output(_))));
    }

    #[test]
    fn command_lines() {
        assert_eq!(
            search_args("maps", 10, Some(Platform::IPad)),
            ["search", "maps", "--limit", "10", "--platform", "ipad"]
        );
        assert_eq!(
            versions_args(&AppRef::Id(111), None),
            ["list-versions", "--app-id", "111"]
        );
        let request = DownloadRequest {
            app: AppRef::BundleId("com.example.one".into()),
            version: Some("800000001".into()),
            output: Some(PathBuf::from("out")),
            purchase: true,
            platform: None,
        };
        assert_eq!(
            download_args(&request),
            [
                "download",
                "--bundle-identifier",
                "com.example.one",
                "--external-version-id",
                "800000001",
                "--output",
                "out",
                "--purchase"
            ]
        );
    }
}

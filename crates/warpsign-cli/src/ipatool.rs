//! The App Store through the `ipatool` program
//! (https://github.com/majd/ipatool), which warpsign runs rather than
//! bundles: a newer ipatool needs no new warpsign.
//!
//! ipatool keeps its own App Store session (in the system keychain, or a
//! passphrase-protected file where there's none). warpsign never sees the
//! Apple ID password: `store login` hands the terminal to ipatool.

use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use warpsign_core::appstore::{
    AppRef, AppStore, Download, DownloadRequest, Platform, StoreAccount, StoreApp, StoreError,
    Versions, ipatool,
};

pub const INSTALL_HINT: &str = "get it from https://github.com/majd/ipatool/releases \
     (or `brew install ipatool`), or point --ipatool at it";

pub struct Ipatool {
    program: PathBuf,
    /// For ipatool's file keychain. Passed on its command line, as ipatool
    /// takes it no other way when it isn't attached to a terminal.
    passphrase: Mutex<Option<String>>,
}

impl Ipatool {
    pub fn new(program: Option<PathBuf>, passphrase: Option<String>) -> Self {
        Self {
            program: program.unwrap_or_else(|| "ipatool".into()),
            passphrase: Mutex::new(passphrase.filter(|p| !p.is_empty())),
        }
    }

    fn command(&self) -> Command {
        Command::new(&self.program)
    }

    fn spawn_error(&self, error: std::io::Error) -> anyhow::Error {
        if error.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("ipatool isn't installed: {INSTALL_HINT}")
        } else {
            anyhow::Error::new(error).context(format!("running {}", self.program.display()))
        }
    }

    /// Run with JSON output, and ask for the keychain passphrase once if
    /// ipatool needs it
    fn json<T>(
        &self,
        args: &[String],
        parse: impl Fn(&str) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let stdout = self.run_json(args)?;
        match parse(&stdout) {
            Err(StoreError::KeychainLocked) if self.ask_passphrase()? => {
                parse(&self.run_json(args)?)
            }
            other => other,
        }
    }

    fn run_json(&self, args: &[String]) -> Result<String, StoreError> {
        let mut command = self.command();
        command.args(["--format", "json", "--non-interactive"]);
        if let Some(passphrase) = self.passphrase.lock().unwrap().as_ref() {
            command.arg("--keychain-passphrase").arg(passphrase);
        }
        let output = command
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| StoreError::Failed(format!("{:#}", self.spawn_error(error))))?;
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() && stdout.trim().is_empty() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(StoreError::Failed(format!(
                "ipatool failed ({}): {}",
                output.status,
                stderr.trim()
            )));
        }
        Ok(stdout)
    }

    /// Whether there's now a passphrase to try
    fn ask_passphrase(&self) -> Result<bool, StoreError> {
        let mut passphrase = self.passphrase.lock().unwrap();
        if passphrase.is_some() || !std::io::stdin().is_terminal() {
            return Ok(false);
        }
        let entered = rpassword::prompt_password(
            "Passphrase for ipatool's keychain (not your Apple ID password): ",
        )
        .map_err(|error| StoreError::Failed(error.to_string()))?;
        if entered.is_empty() {
            return Ok(false);
        }
        *passphrase = Some(entered);
        Ok(true)
    }

    /// Run with the terminal handed over, for ipatool's own prompts
    fn interactive(&self, args: &[OsString]) -> Result<()> {
        let mut command = self.command();
        if let Some(passphrase) = self.passphrase.lock().unwrap().as_ref() {
            command.arg("--keychain-passphrase").arg(passphrase);
        }
        let status = command
            .args(args)
            .status()
            .map_err(|error| self.spawn_error(error))?;
        if !status.success() {
            bail!("ipatool {} failed ({status})", args[0].to_string_lossy());
        }
        Ok(())
    }

    /// Sign in with ipatool's own prompts (Apple ID, password, two-factor
    /// code, and its keychain's passphrase where it needs one)
    pub fn login(&self, email: Option<&str>) -> Result<()> {
        let mut args: Vec<OsString> = vec!["auth".into(), "login".into()];
        if let Some(email) = email {
            args.extend(["--email".into(), email.into()]);
        }
        self.interactive(&args)
    }

    pub fn logout(&self) -> Result<()> {
        self.json(&["auth".into(), "revoke".into()], |out| {
            ipatool::result(out, "success").map(|_| ())
        })
        .context("signing out of ipatool")
    }
}

impl AppStore for Ipatool {
    fn account(&self) -> Result<StoreAccount, StoreError> {
        self.json(&["auth".into(), "info".into()], ipatool::parse_account)
    }

    fn search(
        &self,
        term: &str,
        limit: u32,
        platform: Option<Platform>,
    ) -> Result<Vec<StoreApp>, StoreError> {
        self.json(
            &ipatool::search_args(term, limit, platform),
            ipatool::parse_search,
        )
    }

    fn versions(&self, app: &AppRef, platform: Option<Platform>) -> Result<Versions, StoreError> {
        self.json(
            &ipatool::versions_args(app, platform),
            ipatool::parse_versions,
        )
    }

    fn download(&self, request: &DownloadRequest) -> Result<Download, StoreError> {
        self.json(&ipatool::download_args(request), ipatool::parse_download)
    }
}

//! `warpsign`: sign iOS apps with your Apple Developer account from Linux,
//! Windows or macOS.

mod store;
mod transport;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use plist::{Dictionary, Value as PlistValue};
use warpsign_core::capability_names::{self, EntitlementNames};
use warpsign_core::device::{self, Target};
use warpsign_core::identity::{self, CertificateKind, Identity};
use warpsign_core::info_plist::{PatchingOptions, StatusBarStyle, UiStyle};
use warpsign_core::pipeline::{self, ChangeKind, SignOptions, SignedComponent};
use warpsign_core::portal::api::Team;
use warpsign_core::portal::{AuthError, Authenticator, Portal, SignInOutcome, TwoFactorChallenge};
use warpsign_core::profile::ProvisioningProfile;
use warpsign_core::progress::{ProgressObserver, Step};

use store::{Store, write_private};
use transport::UreqTransport;

#[derive(Parser)]
#[command(
    name = "warpsign",
    version,
    about = "Sign iOS apps with your Apple Developer account"
)]
struct Cli {
    /// Where to keep the session and signing identities
    #[arg(long, global = true, env = "WARPSIGN_CONFIG_DIR")]
    config_dir: Option<PathBuf>,
    /// Append every request to Apple and its response to this file, with
    /// cookies, tokens and sign-in values redacted. For bug reports.
    #[arg(long, global = true, env = "WARPSIGN_HTTP_LOG")]
    http_log: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in with your Apple ID (paid Apple Developer Program accounts only)
    Login {
        #[arg(long)]
        apple_id: Option<String>,
        /// Team ID to use, if you're in more than one team
        #[arg(long)]
        team: Option<String>,
    },
    /// Forget the signed-in session
    Logout,
    /// Show the account, team and signing identities, checked with Apple
    Status,
    /// Manage the signing certificates
    #[command(subcommand)]
    Identity(IdentityCommand),
    /// The team's devices, which signed apps install on
    #[command(subcommand)]
    Device(DeviceCommand),
    /// Show what's inside an IPA
    Inspect {
        ipa: PathBuf,
        /// Also list each component's entitlements
        #[arg(short, long)]
        entitlements: bool,
    },
    /// List the Developer Portal's capabilities for the team, with the
    /// entitlement keys each one grants (and names)
    Capabilities {
        /// Print Apple's response as it came, instead
        #[arg(long)]
        json: bool,
    },
    /// Check provisioning profiles, e.g. for PPQ
    Profile {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Re-sign an IPA for your team
    Sign(SignArgs),
    /// Install a signed IPA on a device connected by USB
    Install {
        ipa: PathBuf,
        /// Which device, if more than one is connected
        #[arg(long)]
        udid: Option<String>,
    },
    /// Make a pairing file for WarpSign on a USB-connected iPhone or iPad,
    /// so it can install apps on itself (through LocalDevVPN)
    Pair {
        /// Which device, if more than one is connected
        #[arg(long)]
        udid: Option<String>,
        /// Where to write it [default: <UDID>.rppairing.plist, or
        /// <UDID>.mobiledevicepairing with --lockdown]
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Make a lockdown pairing file, for iOS before 17.4 (iOS 26.6 and
        /// later refuse these over the VPN)
        #[arg(long)]
        lockdown: bool,
    },
}

#[derive(Subcommand)]
enum IdentityCommand {
    /// Create development and distribution certificates (keys made here)
    Create,
    /// Back up every identity as one .p12, protected by a generated passphrase
    Export { file: PathBuf },
    /// Restore identities from a .p12 backup (or any .p12 with a key)
    Import { file: PathBuf },
    /// Forget an identity here. Never revokes the certificate.
    Remove { kind: Kind },
}

#[derive(Subcommand)]
enum DeviceCommand {
    /// List the enabled iPhones and iPads
    List,
    /// Register a device by UDID (find it in Finder or Apple Configurator)
    Add { name: String, udid: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Development,
    Distribution,
}

impl From<Kind> for CertificateKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Development => CertificateKind::Development,
            Kind::Distribution => CertificateKind::Distribution,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum StatusBar {
    Hidden,
    Light,
    Dark,
}

#[derive(Clone, Copy, ValueEnum)]
enum Style {
    Light,
    Dark,
}

/// Python warpsign's options, with the same names
#[derive(Args)]
struct SignArgs {
    ipa: PathBuf,
    /// Where to write the signed IPA [default: <name>-signed.ipa]
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Sign with the distribution certificate (Ad Hoc profiles)
    #[arg(long)]
    distribution: bool,
    /// Change the app's visible name
    #[arg(long)]
    bundle_name: Option<String>,
    /// Change the app's icon (PNG or JPEG, ideally square)
    #[arg(long)]
    icon: Option<PathBuf>,
    /// Keep identifiers as they are (only for apps you own)
    #[arg(long)]
    no_encode_ids: bool,
    /// Don't replace IDs inside binaries and plists
    #[arg(long)]
    no_patch_ids: bool,
    /// Keep the original bundle ID. Refused with PPQ profiles.
    #[arg(long)]
    force_original_id: bool,
    /// Allow debuggers (get-task-allow)
    #[arg(long)]
    patch_debug: bool,
    /// Support every device and lower the minimum iOS version
    #[arg(long)]
    patch_all_devices: bool,
    /// Enable Files app and iTunes file sharing
    #[arg(long)]
    patch_file_sharing: bool,
    /// Enable ProMotion (120 Hz)
    #[arg(long)]
    patch_promotion: bool,
    /// Force fullscreen on iPad
    #[arg(long)]
    patch_fullscreen: bool,
    /// Allow every orientation
    #[arg(long)]
    patch_orientation: bool,
    /// Enable Game Mode
    #[arg(long)]
    patch_game_mode: bool,
    #[arg(long, value_enum)]
    patch_status_bar: Option<StatusBar>,
    #[arg(long, value_enum)]
    patch_user_interface_style: Option<Style>,
    /// Remove URL scheme registrations
    #[arg(long)]
    remove_url_schemes: bool,
    /// List what each component was signed with, and what changed
    #[arg(short, long)]
    entitlements: bool,
    /// Then install it on a device connected by USB
    #[arg(long)]
    install: bool,
    /// Which device to install on, if more than one is connected
    #[arg(long, requires = "install")]
    udid: Option<String>,
}

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run(cli) {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

/// Set once from `--http-log`
static HTTP_LOG: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn transport() -> Result<Arc<UreqTransport>> {
    let transport = UreqTransport::new();
    Ok(Arc::new(match HTTP_LOG.get() {
        Some(path) => transport
            .with_log(path)
            .with_context(|| format!("opening {}", path.display()))?,
        None => transport,
    }))
}

fn run(cli: Cli) -> Result<()> {
    if let Some(path) = cli.http_log {
        let _ = HTTP_LOG.set(path);
    }
    let store = Store::open(cli.config_dir)?;
    match cli.command {
        Command::Login { apple_id, team } => login(&store, apple_id, team),
        Command::Logout => {
            store.forget_session()?;
            println!(
                "Signed out. Signing identities are kept in {}.",
                store.dir().display()
            );
            Ok(())
        }
        Command::Status => status(&store),
        Command::Identity(command) => identity_command(&store, command),
        Command::Device(command) => device_command(&store, command),
        Command::Inspect { ipa, entitlements } => inspect(&store, &ipa, entitlements),
        Command::Capabilities { json } => capabilities(&store, json),
        Command::Profile { files } => profiles(&files),
        Command::Sign(args) => sign(&store, args),
        Command::Install { ipa, udid } => install(&ipa, udid),
        Command::Pair {
            udid,
            output,
            lockdown,
        } => pair(udid, output, lockdown),
    }
}

// Signing in

fn prompt(question: &str) -> Result<String> {
    eprint!("{question}");
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn login(store: &Store, apple_id: Option<String>, team: Option<String>) -> Result<()> {
    let mut account = store.account();
    let apple_id = match apple_id.or(account.apple_id.clone()) {
        Some(id) => id,
        None => prompt("Apple ID: ")?,
    };
    let password = rpassword::prompt_password(format!("Password for {apple_id}: "))?;

    // The last session's two-factor trust, so Apple needn't ask for a code
    let trust = store
        .session()
        .map(|session| session.trust())
        .unwrap_or_default();
    let auth = Authenticator::with_session(transport()?, trust);
    let outcome = auth.sign_in(&apple_id, &password);
    if let Err(AuthError::Refused(_)) = &outcome {
        eprintln!("Don't retry straight away: repeated attempts can lock your Apple ID.");
    }
    let teams = match outcome? {
        SignInOutcome::SignedIn { teams } => teams,
        SignInOutcome::NeedsCode(challenge) => two_factor(&auth, &challenge)?,
    };
    store.save_session(&auth.session())?;

    let team = choose_team(&teams, team.as_deref().or(account.team_id.as_deref()))?;
    account.apple_id = Some(apple_id);
    account.team_id = Some(team.team_id.clone());
    account.team_name = Some(team.name.clone());
    store.save_account(&account)?;
    println!("Signed in to {} ({}).", team.name, team.team_id);
    if teams.len() > 1 {
        println!("Use `warpsign login --team <ID>` to switch teams.");
    }
    Ok(())
}

fn two_factor(auth: &Authenticator, challenge: &TwoFactorChallenge) -> Result<Vec<Team>> {
    let mut phone = challenge.sms_sent_to.clone();
    match &phone {
        Some(sent) => eprintln!("A code was sent to {}.", sent.number),
        None if challenge.trusted_devices => {
            eprintln!("Enter the code shown on your Apple devices.")
        }
        None => {}
    }
    if !challenge.phone_numbers.is_empty() {
        eprintln!("Type `sms` to get a code by text message instead.");
    }
    for _ in 0..5 {
        let code = prompt("Code: ")?;
        if code.eq_ignore_ascii_case("sms") {
            for (index, number) in challenge.phone_numbers.iter().enumerate() {
                eprintln!("  {}. {}", index + 1, number.number);
            }
            let choice = if challenge.phone_numbers.len() == 1 {
                1
            } else {
                prompt("Text which number? ")?.parse::<usize>().unwrap_or(0)
            };
            let Some(chosen) = choice
                .checked_sub(1)
                .and_then(|i| challenge.phone_numbers.get(i))
            else {
                eprintln!("No such number.");
                continue;
            };
            auth.request_sms(chosen.id)?;
            eprintln!("Sent a code to {}.", chosen.number);
            phone = Some(chosen.clone());
            continue;
        }
        let result = match &phone {
            Some(phone) => auth.verify_sms_code(phone.id, &code),
            None => auth.verify_device_code(&code),
        };
        match result {
            Err(AuthError::IncorrectCode(message)) => eprintln!("{message}"),
            other => return Ok(other?),
        }
    }
    bail!("too many incorrect codes")
}

fn choose_team<'a>(teams: &'a [Team], wanted: Option<&str>) -> Result<&'a Team> {
    if let Some(wanted) = wanted {
        if let Some(team) = teams.iter().find(|team| team.team_id == wanted) {
            return Ok(team);
        }
        eprintln!("Team {wanted} isn't one of this Apple ID's teams.");
    }
    if teams.len() == 1 {
        return Ok(&teams[0]);
    }
    for (index, team) in teams.iter().enumerate() {
        eprintln!(
            "  {}. {} ({}, {})",
            index + 1,
            team.name,
            team.team_id,
            team.kind
        );
    }
    let choice: usize = prompt("Use which team? ")?.parse().unwrap_or(0);
    choice
        .checked_sub(1)
        .and_then(|index| teams.get(index))
        .context("no such team")
}

/// The saved session, for the saved team
fn portal(store: &Store) -> Result<(Authenticator, Portal)> {
    let session = store
        .session()
        .context("not signed in: run `warpsign login`")?;
    let team = store
        .account()
        .team_id
        .context("no team chosen: run `warpsign login`")?;
    let auth = Authenticator::with_session(transport()?, session);
    let portal = Portal::new(auth.http(), &team);
    Ok((auth, portal))
}

fn status(store: &Store) -> Result<()> {
    let account = store.account();
    let identities = store.identities()?;
    match store.session() {
        None => println!("Not signed in."),
        Some(_) => {
            let (auth, portal) = portal(store)?;
            match auth.restore() {
                Ok(teams) => {
                    store.save_session(&auth.session())?;
                    println!(
                        "Signed in as {} to {} ({}). {} team{} available.",
                        account.apple_id.as_deref().unwrap_or("?"),
                        account.team_name.as_deref().unwrap_or("?"),
                        portal.team_id(),
                        teams.len(),
                        if teams.len() == 1 { "" } else { "s" }
                    );
                    if !identities.is_empty() {
                        let certificates = portal.certificates()?;
                        for identity in &identities {
                            let found = certificates
                                .iter()
                                .any(|c| identity.has_serial(&c.serial_number));
                            print_identity(identity, Some(found));
                        }
                    }
                    return Ok(());
                }
                Err(error) => println!("Signed out: {error}"),
            }
        }
    }
    for identity in &identities {
        print_identity(identity, None);
    }
    if identities.is_empty() {
        println!(
            "No signing identities: run `warpsign identity create` or `warpsign identity import`."
        );
    }
    Ok(())
}

fn print_identity(identity: &Identity, on_portal: Option<bool>) {
    let state = match on_portal {
        Some(true) => ", active",
        Some(false) => ", NOT on the portal (revoked or expired)",
        None => "",
    };
    println!(
        "  {} (serial {}{state})",
        identity.common_name().unwrap_or_else(|| "?".into()),
        identity.serial_number()
    );
}

// Identities

fn identity_command(store: &Store, command: IdentityCommand) -> Result<()> {
    match command {
        IdentityCommand::Create => {
            let (auth, portal) = portal(store)?;
            for kind in [CertificateKind::Development, CertificateKind::Distribution] {
                if store.identity(kind)?.is_some() {
                    println!("Already have a {kind:?} identity.");
                    continue;
                }
                let key = identity::generate_private_key()?;
                let csr = identity::certificate_signing_request(&key, "WarpSign")?;
                match portal.create_certificate(kind, &csr) {
                    Ok((_, certificate)) => {
                        let created = Identity::new(&certificate, &key)?;
                        store.save_identity(kind, &created)?;
                        print_identity(&created, Some(true));
                    }
                    // Teams get few distribution slots; development is enough to sign
                    Err(error) if kind == CertificateKind::Distribution => {
                        println!("No distribution certificate: {error}");
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            store.save_session(&auth.session())?;
            println!("Back these up with `warpsign identity export <file.p12>`.");
            Ok(())
        }
        IdentityCommand::Export { file } => {
            let identities = store.identities()?;
            if identities.is_empty() {
                bail!("no identities to export");
            }
            let passphrase = identity::generate_passphrase();
            let p12 = identity::export_p12(&identities, &passphrase)?;
            write_private(&file, &p12).with_context(|| format!("writing {}", file.display()))?;
            println!(
                "Wrote {} identit{} to {}.",
                identities.len(),
                if identities.len() == 1 { "y" } else { "ies" },
                file.display()
            );
            println!("Passphrase: {passphrase}");
            println!(
                "Keep both somewhere safe, like a password manager: they let anyone sign as your team."
            );
            Ok(())
        }
        IdentityCommand::Import { file } => {
            let data =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            // From the environment for automation, otherwise asked for
            let password = match std::env::var("WARPSIGN_P12_PASSPHRASE") {
                Ok(password) => password,
                Err(_) => rpassword::prompt_password("Passphrase: ")?,
            };
            let mut imported = 0;
            for identity in identity::import_p12(&data, &password)? {
                match identity.kind() {
                    Some(kind) => {
                        store.save_identity(kind, &identity)?;
                        print_identity(&identity, None);
                        imported += 1;
                    }
                    None => println!(
                        "Skipped {}: not an Apple Development or Distribution certificate",
                        identity.common_name().unwrap_or_default()
                    ),
                }
            }
            if imported == 0 {
                bail!("no usable identities in {}", file.display());
            }
            Ok(())
        }
        IdentityCommand::Remove { kind } => {
            store.remove_identity(kind.into())?;
            println!("Removed. The certificate wasn't revoked.");
            Ok(())
        }
    }
}

fn device_command(store: &Store, command: DeviceCommand) -> Result<()> {
    let (auth, portal) = portal(store)?;
    let result = match command {
        DeviceCommand::List => portal.devices().map(|devices| {
            for device in &devices {
                println!(
                    "{:<28} {:<7} {}{}",
                    device.name,
                    device.device_class,
                    device.udid,
                    device
                        .model
                        .as_deref()
                        .map(|m| format!("  ({m})"))
                        .unwrap_or_default()
                );
            }
            if devices.is_empty() {
                println!("No iPhones or iPads registered: add one with `warpsign device add`.");
            }
        }),
        DeviceCommand::Add { name, udid } => portal
            .register_device(&name, &udid)
            .map(|device| println!("Registered {} ({})", device.name, device.udid)),
    };
    store.save_session(&auth.session())?;
    Ok(result?)
}

// Apps

/// Prints steps as they run
#[derive(Default)]
struct Printer {
    /// Last tenth of progress printed
    shown: std::sync::atomic::AtomicU32,
}

impl ProgressObserver for Printer {
    fn step_progress(&self, _step: Step, fraction: f64) {
        use std::sync::atomic::Ordering;
        let tenth = (fraction * 10.0).floor() as u32;
        if tenth > self.shown.swap(tenth, Ordering::Relaxed) {
            eprintln!("    {}%", tenth * 10);
        }
    }
    fn step_started(&self, step: Step) {
        self.shown.store(0, std::sync::atomic::Ordering::Relaxed);
        eprintln!("▸ {}", step.title());
    }
    fn step_log(&self, _step: Step, message: &str) {
        eprintln!("    {message}");
    }
    fn step_finished(&self, step: Step, elapsed: Duration, error: Option<&str>) {
        match error {
            None => eprintln!("✓ {} ({:.1} s)", step.title(), elapsed.as_secs_f64()),
            Some(error) => eprintln!("✗ {}: {error}", step.title()),
        }
    }
}

/// Fetch the team's capabilities response, keeping it for when the portal
/// can't be reached
fn fetch_capabilities(store: &Store) -> Result<serde_json::Value> {
    let (auth, portal) = portal(store)?;
    let result = portal.capabilities();
    store.save_session(&auth.session())?;
    let response = result?;
    store.save_capabilities(portal.team_id(), &response)?;
    Ok(response)
}

/// Names for entitlement keys: from the portal when signed in, otherwise
/// from the last capabilities response fetched for the team
fn entitlement_names(store: &Store) -> EntitlementNames {
    let response = match fetch_capabilities(store) {
        Ok(response) => Some(response),
        Err(error) => {
            let cached = store
                .account()
                .team_id
                .and_then(|team| store.capabilities(&team));
            if store.session().is_some() {
                eprintln!(
                    "note: couldn't fetch capabilities ({error:#}){}",
                    if cached.is_some() {
                        "; using the saved ones"
                    } else {
                        ""
                    }
                );
            }
            cached
        }
    };
    response
        .map(|response| EntitlementNames::from_capabilities(&response))
        .unwrap_or_else(EntitlementNames::without_portal)
}

fn capabilities(store: &Store, json: bool) -> Result<()> {
    let response = fetch_capabilities(store)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }
    // The same reading of the response that names entitlements
    let listed = capability_names::capabilities(&response);
    let mut keys = std::collections::BTreeSet::new();
    for capability in &listed {
        println!(
            "{:<40} {}",
            capability.id.as_deref().unwrap_or("(no ID)"),
            capability.name.as_deref().unwrap_or("(no name)")
        );
        for key in &capability.entitlement_keys {
            println!("    {key}");
            keys.insert(key.as_str());
        }
    }
    println!(
        "{} capabilities, {} entitlement keys",
        listed.len(),
        keys.len()
    );
    Ok(())
}

fn inspect(store: &Store, ipa: &Path, show_entitlements: bool) -> Result<()> {
    let work = tempfile::tempdir()?;
    let app = pipeline::inspect_ipa(ipa, work.path(), &Printer::default())?;
    let names = if show_entitlements {
        entitlement_names(store)
    } else {
        EntitlementNames::without_portal()
    };
    for component in &app.components {
        let path = warpsign_core::inspect::slash_path(&component.path);
        println!(
            "{:<40} {}{}",
            if path.is_empty() { "App".into() } else { path },
            component.bundle_id,
            if component.is_primary {
                "  [App ID]"
            } else {
                ""
            }
        );
        if show_entitlements {
            if component.entitlements.is_empty() {
                println!("    (no entitlements)");
            }
            for (key, value) in &component.entitlements {
                let name = names
                    .get(key)
                    .map(|name| format!(" ({})", name.name))
                    .unwrap_or_default();
                println!("    {key}{name} = {}", plist_text(value));
            }
        }
    }
    Ok(())
}

/// A plist value on one line
fn plist_text(value: &PlistValue) -> String {
    match value {
        PlistValue::Boolean(value) => value.to_string(),
        PlistValue::String(value) => value.clone(),
        PlistValue::Integer(value) => value.to_string(),
        PlistValue::Real(value) => value.to_string(),
        PlistValue::Array(items) => format!(
            "[{}]",
            items.iter().map(plist_text).collect::<Vec<_>>().join(", ")
        ),
        PlistValue::Dictionary(dict) => format!(
            "{{{}}}",
            dict.iter()
                .map(|(key, value)| format!("{key}: {}", plist_text(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PlistValue::Data(data) => format!("<{} bytes>", data.len()),
        PlistValue::Date(date) => date.to_xml_format(),
        PlistValue::Uid(uid) => format!("UID {}", uid.get()),
        _ => "?".into(),
    }
}

/// What each component was signed with, and how that differs from before
fn print_signed_components(components: &[SignedComponent]) {
    for component in components {
        if component.original_entitlements.is_empty() && component.entitlements.is_empty() {
            continue;
        }
        let path = if component.path.is_empty() {
            "App"
        } else {
            &component.path
        };
        if component.bundle_id == component.original_bundle_id {
            println!("{path} ({})", component.bundle_id);
        } else {
            println!(
                "{path} ({} → {})",
                component.original_bundle_id, component.bundle_id
            );
        }
        let name = |key: &str| {
            component
                .names
                .get(key)
                .map(|name| format!(" ({})", name.name))
                .unwrap_or_default()
        };
        let value =
            |dict: &Dictionary, key: &str| dict.get(key).map(plist_text).unwrap_or_default();
        if component.is_primary {
            println!("  Signed with:");
            for (key, signed) in &component.entitlements {
                println!("    {key}{} = {}", name(key), plist_text(signed));
            }
        }
        if !component.capabilities.is_empty() {
            println!("  Capabilities: {}", component.capabilities.join(", "));
        }
        if !component.changes.is_empty() {
            println!("  Changes:");
        }
        for change in &component.changes {
            let key = &change.key;
            let text = match change.kind {
                ChangeKind::Added => format!(
                    "+ {key}{} = {}",
                    name(key),
                    value(&component.entitlements, key)
                ),
                ChangeKind::Removed => format!("- {key}{}", name(key)),
                ChangeKind::Changed => format!(
                    "~ {key}{}: {} → {}",
                    name(key),
                    value(&component.original_entitlements, key),
                    value(&component.entitlements, key)
                ),
            };
            match change.reason {
                Some(reason) => println!("    {text}: {reason}"),
                None => println!("    {text}"),
            }
        }
    }
}

fn profiles(files: &[PathBuf]) -> Result<()> {
    for file in files {
        let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
        let profile = ProvisioningProfile::parse(&data)
            .with_context(|| format!("reading {}", file.display()))?;
        println!(
            "{}: {} (team {}), {} devices, PPQ {}{}",
            file.display(),
            profile.name.as_deref().unwrap_or("unnamed"),
            profile.team_ids.join(", "),
            profile.provisioned_devices.len(),
            if profile.ppq_check { "ON" } else { "off" },
            if profile.is_wildcard() {
                ", wildcard (not supported)"
            } else {
                ""
            },
        );
    }
    Ok(())
}

fn sign(store: &Store, args: SignArgs) -> Result<()> {
    let kind = if args.distribution {
        CertificateKind::Distribution
    } else {
        CertificateKind::Development
    };
    let identity = store
        .identity(kind)?
        .with_context(|| format!("no {kind:?} identity: run `warpsign identity create`"))?;
    let (auth, portal) = portal(store)?;

    let output = args.output.clone().unwrap_or_else(|| {
        let stem = args.ipa.file_stem().unwrap_or_default().to_string_lossy();
        args.ipa.with_file_name(format!("{stem}-signed.ipa"))
    });
    let icon = match &args.icon {
        Some(path) => {
            Some(std::fs::read(path).with_context(|| format!("reading {}", path.display()))?)
        }
        None => None,
    };
    let options = SignOptions {
        patching: PatchingOptions {
            icon,
            patch_ids: !args.no_patch_ids,
            force_original_id: args.force_original_id,
            bundle_name: args.bundle_name,
            patch_all_devices: args.patch_all_devices,
            patch_file_sharing: args.patch_file_sharing,
            patch_promotion: args.patch_promotion,
            patch_fullscreen: args.patch_fullscreen,
            patch_orientation: args.patch_orientation,
            patch_game_mode: args.patch_game_mode,
            patch_status_bar: match args.patch_status_bar {
                None => StatusBarStyle::Default,
                Some(StatusBar::Hidden) => StatusBarStyle::Hidden,
                Some(StatusBar::Light) => StatusBarStyle::Light,
                Some(StatusBar::Dark) => StatusBarStyle::Dark,
            },
            patch_user_interface_style: match args.patch_user_interface_style {
                None => UiStyle::Automatic,
                Some(Style::Light) => UiStyle::Light,
                Some(Style::Dark) => UiStyle::Dark,
            },
            remove_url_schemes: args.remove_url_schemes,
        },
        encode_ids: !args.no_encode_ids,
        patch_debug: args.patch_debug,
        // Profiles kept between signs, so an unchanged app makes no portal changes
        profile_cache: Some(store.dir().join("profiles")),
        pretend_not_granted: false,
    };

    let work = tempfile::tempdir()?;
    let result = pipeline::sign_ipa(
        &portal,
        &identity,
        &args.ipa,
        &output,
        work.path(),
        &options,
        &Printer::default(),
    );
    // Keep refreshed cookies and tokens, even if signing failed
    store.save_session(&auth.session())?;
    let signed = result?;
    store.save_capabilities(portal.team_id(), &signed.capabilities_response)?;
    if args.entitlements {
        print_signed_components(&signed.components);
    }
    for warning in &signed.warnings {
        eprintln!("warning: {warning}");
    }
    println!("Signed {} as {}", output.display(), signed.bundle_id);
    if args.install {
        install(&output, args.udid)?;
    }
    Ok(())
}

fn install(ipa: &Path, udid: Option<String>) -> Result<()> {
    device::install_ipa(&Target::Usb { udid }, ipa, &Printer::default())?;
    println!("Installed {}", ipa.display());
    Ok(())
}

fn pair(udid: Option<String>, output: Option<PathBuf>, lockdown: bool) -> Result<()> {
    let devices = device::usb_devices()?;
    let udid = match (udid, devices.as_slice()) {
        (Some(udid), _) => udid,
        (None, []) => bail!("no iPhone or iPad is connected by USB"),
        (None, [only]) => only.clone(),
        (None, _) => bail!(
            "more than one device is connected; pick one with --udid: {}",
            devices.join(", ")
        ),
    };
    eprintln!("Unlock the device, and tap Trust if it asks.");
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "warpsign".into());
    let kind = if lockdown {
        device::PairingKind::Lockdown
    } else {
        device::PairingKind::RemotePairing
    };
    let (udid, data) = device::create_pairing_file(Some(&udid), &host, kind)?;
    let output = output.unwrap_or_else(|| {
        PathBuf::from(if lockdown {
            format!("{udid}.mobiledevicepairing")
        } else {
            format!("{udid}.rppairing.plist")
        })
    });
    write_private(&output, &data).with_context(|| format!("writing {}", output.display()))?;
    println!("Wrote {}", output.display());
    println!("AirDrop it to the device and open it in WarpSign (Settings > Installing).");
    println!("It lets apps on that device install apps: keep it private.");
    Ok(())
}

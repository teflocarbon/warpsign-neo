//! Reusing provisioning profiles instead of regenerating them on every
//! sign, so signing the same app again makes no changes on the portal.
//!
//! Not part of Python warpsign, which re-registers every identifier, sets
//! every App ID's capabilities and regenerates every profile on each sign:
//! dozens of writes, enough for Apple to rate-limit busy accounts. A
//! profile is the portal's own record of what its App ID grants, so when
//! the existing one is active, has this certificate and every device, and
//! grants every entitlement about to be signed, the App ID needs no
//! changes and the profile can be used as it is. Anything else, and the
//! pipeline updates the App ID and regenerates the profile as before.
//!
//! Profiles downloaded or made are kept in a cache directory, keyed by the
//! portal's profile ID and checked against the expiry the portal lists, so
//! an unchanged profile isn't downloaded again either.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use plist::{Dictionary, Value};

use crate::profile::ProvisioningProfile;

/// Profiles closer to expiry than this are regenerated rather than reused
pub const MINIMUM_VALIDITY: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Why a profile can't be reused as it is
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotReusable {
    /// The portal says it's expired or invalid (e.g. its certificate was
    /// revoked, or its App ID changed after it was made)
    State(String),
    Expiring,
    /// It's for another App ID
    AppId(String),
    Certificate,
    /// Devices registered since it was made
    Devices(usize),
    /// An entitlement about to be signed that it doesn't grant
    Entitlement(String),
}

impl NotReusable {
    /// Whether the App ID's capabilities may need changing too, not just
    /// the profile regenerating
    pub fn needs_capabilities(&self) -> bool {
        matches!(
            self,
            NotReusable::State(_) | NotReusable::AppId(_) | NotReusable::Entitlement(_)
        )
    }
}

impl std::fmt::Display for NotReusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotReusable::State(state) => {
                write!(f, "the portal lists it as {}", state.to_lowercase())
            }
            NotReusable::Expiring => write!(f, "it expires within a week"),
            NotReusable::AppId(id) => write!(f, "it's for {id}"),
            NotReusable::Certificate => write!(f, "it doesn't include this certificate"),
            NotReusable::Devices(1) => write!(f, "a device was added since"),
            NotReusable::Devices(count) => write!(f, "{count} devices were added since"),
            NotReusable::Entitlement(key) => write!(f, "it doesn't grant {key}"),
        }
    }
}

/// Whether `profile` can sign a component with `entitlements` as it is:
/// for `app_identifier` (TEAMID.bundle.id), with the certificate
/// `certificate_der`, on every device in `device_udids`
pub fn check(
    profile: &ProvisioningProfile,
    app_identifier: &str,
    certificate_der: &[u8],
    device_udids: &[String],
    entitlements: &Dictionary,
    now: SystemTime,
) -> Result<(), NotReusable> {
    match profile.expiration_date {
        Some(expires) if expires > now + MINIMUM_VALIDITY => {}
        _ => return Err(NotReusable::Expiring),
    }
    let profile_app_id = profile
        .entitlements
        .get("application-identifier")
        .and_then(Value::as_string)
        .unwrap_or_default();
    if profile_app_id != app_identifier {
        return Err(NotReusable::AppId(profile_app_id.to_string()));
    }
    let has_certificate = profile
        .plist
        .get("DeveloperCertificates")
        .and_then(Value::as_array)
        .is_some_and(|certificates| {
            certificates
                .iter()
                .any(|certificate| certificate.as_data() == Some(certificate_der))
        });
    if !has_certificate {
        return Err(NotReusable::Certificate);
    }
    let provisioned: HashSet<&str> = profile
        .provisioned_devices
        .iter()
        .map(String::as_str)
        .collect();
    let missing = device_udids
        .iter()
        .filter(|udid| !provisioned.contains(udid.as_str()))
        .count();
    if missing > 0 {
        return Err(NotReusable::Devices(missing));
    }
    for (key, wanted) in entitlements {
        let granted = profile
            .entitlements
            .get(key)
            .is_some_and(|allowed| grants(allowed, wanted));
        if !granted {
            return Err(NotReusable::Entitlement(key.clone()));
        }
    }
    Ok(())
}

/// The entitlements in `entitlements` that `profile` doesn't grant, which
/// iOS refuses to install an app over ("A valid provisioning profile for
/// this executable was not found")
pub fn ungranted(profile: &ProvisioningProfile, entitlements: &Dictionary) -> Vec<String> {
    entitlements
        .iter()
        .filter(|(key, wanted)| {
            !profile
                .entitlements
                .get(key.as_str())
                .is_some_and(|allowed| grants(allowed, wanted))
        })
        .map(|(key, _)| key.clone())
        .collect()
}

/// Whether a profile's entitlement value allows a signed one. Profiles use
/// `*` as a wildcard (`TEAMID.*`, or `*` alone), list what may be signed
/// in arrays, and use `true` to allow either value of a boolean.
/// Conservative: anything else must match exactly, and an unexpected
/// shape counts as not granted, which only costs a regeneration.
fn grants(allowed: &Value, wanted: &Value) -> bool {
    match (allowed, wanted) {
        (_, Value::Boolean(false)) => true,
        (Value::Boolean(true), Value::Boolean(true)) => true,
        (_, Value::Array(items)) => items.iter().all(|item| grants(allowed, item)),
        (Value::Array(options), wanted) => options.iter().any(|option| grants(option, wanted)),
        (Value::String(pattern), Value::String(value)) => matches(pattern, value),
        (allowed, wanted) => allowed == wanted,
    }
}

/// `*` alone matches anything; a trailing `*` matches any rest
fn matches(pattern: &str, value: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

/// A date as the portal lists it, e.g. "2027-10-11T12:00:00.000+00:00" or
/// "2027-10-11T12:00:00Z"
pub fn portal_date(text: &str) -> Option<SystemTime> {
    let (date, rest) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;

    // The time, then an offset: Z, +hh:mm or -hh:mm
    let offset_at = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (time, offset) = rest.split_at(offset_at);
    let time = time.split('.').next()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next().unwrap_or("0").parse().ok()?;
    let offset_seconds = match offset {
        "" | "Z" => 0,
        offset => {
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let (hours, minutes) = offset[1..].split_once(':').unwrap_or((&offset[1..], "0"));
            sign * (hours.parse::<i64>().ok()? * 3600 + minutes.parse::<i64>().ok()? * 60)
        }
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // Days since 1970 (Howard Hinnant's days_from_civil)
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;

    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_seconds;
    u64::try_from(seconds)
        .ok()
        .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds))
}

/// Downloaded and made profiles, one directory per team
#[derive(Debug, Clone)]
pub struct ProfileCache {
    dir: PathBuf,
}

impl ProfileCache {
    pub fn new(root: &Path, team_id: &str) -> Self {
        Self {
            dir: root.join(safe_name(team_id)),
        }
    }

    fn path(&self, profile_id: &str) -> PathBuf {
        self.dir
            .join(format!("{}.mobileprovision", safe_name(profile_id)))
    }

    /// The cached profile `profile_id`, if it's the one the portal lists:
    /// same expiry, give or take a second or two of rounding. A profile
    /// regenerated elsewhere gets a new expiry, so it isn't mistaken for
    /// this one.
    pub fn load(&self, profile_id: &str, expires: SystemTime) -> Option<Vec<u8>> {
        let content = std::fs::read(self.path(profile_id)).ok()?;
        let cached_expiry = ProvisioningProfile::parse(&content).ok()?.expiration_date?;
        let difference = cached_expiry
            .duration_since(expires)
            .or_else(|_| expires.duration_since(cached_expiry))
            .ok()?;
        (difference <= Duration::from_secs(2)).then_some(content)
    }

    /// Keep a profile. Failing to is harmless: it's downloaded next time.
    pub fn store(&self, profile_id: &str, content: &[u8]) {
        if std::fs::create_dir_all(&self.dir).is_ok() {
            let _ = std::fs::write(self.path(profile_id), content);
        }
    }

    /// Forget profiles the portal no longer lists
    pub fn prune(&self, listed: &HashSet<String>) {
        let listed: HashSet<String> = listed.iter().map(|id| safe_name(id)).collect();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let keep = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| listed.contains(stem));
            if !keep && path.extension().is_some_and(|ext| ext == "mobileprovision") {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Portal IDs are alphanumeric; anything else is dropped so an ID can never
/// name a path outside the cache
fn safe_name(text: &str) -> String {
    text.chars().filter(char::is_ascii_alphanumeric).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(text: &str) -> Value {
        Value::String(text.into())
    }

    fn strings(items: &[&str]) -> Value {
        Value::Array(items.iter().map(|item| string(item)).collect())
    }

    #[test]
    fn reads_portal_dates() {
        let seconds = |text| {
            portal_date(text)
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
        };
        assert_eq!(seconds("1970-01-01T00:00:00Z"), 0);
        assert_eq!(seconds("2027-10-11T12:00:00.000+00:00"), 1_823_256_000);
        assert_eq!(seconds("2027-10-11T14:00:00+02:00"), 1_823_256_000);
        assert_eq!(seconds("2027-10-11T07:30:00-04:30"), 1_823_256_000);
        assert_eq!(seconds("2024-02-29T00:00:00Z"), 1_709_164_800);
        assert_eq!(portal_date("not a date"), None);
        assert_eq!(portal_date("2027-13-01T00:00:00Z"), None);
    }

    #[test]
    fn profiles_grant_with_wildcards_and_lists() {
        assert!(grants(
            &string("ZZZZZ99999.*"),
            &string("ZZZZZ99999.com.example")
        ));
        assert!(!grants(
            &string("ZZZZZ99999.*"),
            &string("AAAAA11111.com.example")
        ));
        assert!(grants(&string("*"), &strings(&["applinks:example.com"])));
        assert!(grants(
            &strings(&["ZZZZZ99999.*", "com.apple.token"]),
            &strings(&["ZZZZZ99999.com.example", "com.apple.token"])
        ));
        // App groups are listed one by one
        let groups = strings(&["group.a", "group.b"]);
        assert!(grants(&groups, &strings(&["group.b"])));
        assert!(!grants(&groups, &strings(&["group.c"])));
        assert!(grants(&string("development"), &string("development")));
        assert!(!grants(&string("development"), &string("production")));
        assert!(grants(&Value::Boolean(true), &Value::Boolean(true)));
        assert!(grants(&Value::Boolean(false), &Value::Boolean(false)));
        assert!(!grants(&Value::Boolean(false), &Value::Boolean(true)));
        assert!(!grants(
            &Value::Integer(1.into()),
            &Value::Integer(2.into())
        ));
    }

    #[test]
    fn caches_profiles_by_id_and_prunes_others() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ProfileCache::new(dir.path(), "ZZZZZ99999");
        let profile = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/profiles/no-ppq.mobileprovision"),
        )
        .unwrap();
        let expires = ProvisioningProfile::parse(&profile)
            .unwrap()
            .expiration_date
            .unwrap();

        assert_eq!(cache.load("PROFILE1", expires), None);
        cache.store("PROFILE1", &profile);
        assert_eq!(cache.load("PROFILE1", expires), Some(profile.clone()));
        // Regenerated elsewhere: a different expiry
        assert_eq!(
            cache.load("PROFILE1", expires + Duration::from_secs(86_400)),
            None
        );
        // IDs can't escape the cache directory
        cache.store("../../escape", &profile);
        assert!(
            dir.path()
                .join("ZZZZZ99999/escape.mobileprovision")
                .exists()
        );

        cache.prune(&HashSet::from(["PROFILE1".to_string()]));
        assert!(cache.load("PROFILE1", expires).is_some());
        assert!(
            !dir.path()
                .join("ZZZZZ99999/escape.mobileprovision")
                .exists()
        );
    }
}

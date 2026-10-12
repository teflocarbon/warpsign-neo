//! AltStore-format app sources ("repos"): JSON listing apps to download.
//!
//! Real-world sources are loose with the format (numbers as strings, sizes as
//! floats, three generations of screenshot formats, version details either in
//! `versions` or on the app itself), so parsing is lenient: an app that
//! can't be understood is skipped with a warning instead of failing the
//! whole source.

use serde_json::{Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("not a JSON app source: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not an app source: {0}")]
    Format(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub name: String,
    pub identifier: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub header_url: Option<String>,
    pub website: Option<String>,
    /// Hex colour, e.g. "#5C7AEA"
    pub tint_color: Option<String>,
    /// Bundle IDs of the apps the source wants shown first
    pub featured_apps: Vec<String>,
    pub apps: Vec<SourceApp>,
    pub news: Vec<NewsItem>,
    /// Apps that were skipped, and why
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceApp {
    pub name: String,
    pub bundle_id: String,
    pub developer_name: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub tint_color: Option<String>,
    pub category: Option<String>,
    pub screenshot_urls: Vec<String>,
    /// Newest first. Never empty.
    pub versions: Vec<AppVersion>,
    /// What the app says it uses (`appPermissions`), as the source lists it
    pub permissions: AppPermissions,
}

/// A source's `appPermissions`: entitlement keys, and privacy permissions
/// with the reason the app gives for each
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppPermissions {
    pub entitlements: Vec<String>,
    /// (Info.plist key, e.g. "NSCameraUsageDescription"; reason)
    pub privacy: Vec<(String, Option<String>)>,
}

impl SourceApp {
    pub fn latest(&self) -> &AppVersion {
        &self.versions[0]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppVersion {
    pub version: String,
    pub build: Option<String>,
    /// As given by the source, usually ISO 8601
    pub date: Option<String>,
    pub notes: Option<String>,
    pub download_url: String,
    pub size: Option<u64>,
    pub min_os: Option<String>,
    pub max_os: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewsItem {
    pub identifier: Option<String>,
    pub title: String,
    pub caption: Option<String>,
    pub date: Option<String>,
    pub image_url: Option<String>,
    pub url: Option<String>,
    pub app_id: Option<String>,
}

/// Parse an AltStore-format source
pub fn parse_source(data: &[u8]) -> Result<Source, SourceError> {
    let value: Value = serde_json::from_slice(data)?;
    let root = value
        .as_object()
        .ok_or_else(|| SourceError::Format("expected a JSON object".into()))?;
    let apps_value = root
        .get("apps")
        .and_then(Value::as_array)
        .ok_or_else(|| SourceError::Format("no \"apps\" list".into()))?;

    let mut warnings = Vec::new();
    let mut apps = Vec::new();
    for (index, app) in apps_value.iter().enumerate() {
        match parse_app(app) {
            Ok(app) => apps.push(app),
            Err(reason) => {
                let name = app
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|name| format!("\"{name}\""))
                    .unwrap_or_else(|| format!("#{}", index + 1));
                warnings.push(format!("skipped app {name}: {reason}"));
            }
        }
    }

    let news = root
        .get("news")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|item| {
            Some(NewsItem {
                title: text(item, "title")?,
                identifier: text(item, "identifier"),
                caption: text(item, "caption"),
                date: text(item, "date"),
                image_url: text(item, "imageURL"),
                url: text(item, "url"),
                app_id: text(item, "appID"),
            })
        })
        .collect();

    Ok(Source {
        name: text(root, "name").unwrap_or_else(|| "Untitled source".into()),
        identifier: text(root, "identifier"),
        subtitle: text(root, "subtitle"),
        description: text(root, "description"),
        icon_url: text(root, "iconURL"),
        header_url: text(root, "headerURL"),
        website: text(root, "website"),
        tint_color: text(root, "tintColor").map(hex_color),
        featured_apps: strings(root.get("featuredApps")),
        apps,
        news,
        warnings,
    })
}

fn parse_app(value: &Value) -> Result<SourceApp, String> {
    let app = value.as_object().ok_or("not an object")?;
    let name = text(app, "name").ok_or("no name")?;
    let bundle_id = text(app, "bundleIdentifier").ok_or("no bundleIdentifier")?;

    let mut versions: Vec<AppVersion> = app
        .get("versions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter_map(|version| parse_version(version, None))
        .collect();
    if versions.is_empty() {
        // Legacy format: the one version's details are on the app
        let legacy = parse_version(app, Some("versionDate"));
        versions.extend(legacy);
    }
    if versions.is_empty() {
        return Err("no version with a downloadURL".into());
    }

    Ok(SourceApp {
        name,
        bundle_id,
        developer_name: text(app, "developerName"),
        subtitle: text(app, "subtitle"),
        description: text(app, "localizedDescription"),
        icon_url: text(app, "iconURL"),
        tint_color: text(app, "tintColor").map(hex_color),
        category: text(app, "category"),
        screenshot_urls: screenshots(app),
        versions,
        permissions: permissions(app),
    })
}

/// `appPermissions`: `entitlements` as keys or `{name}` objects, and
/// `privacy` as a key → reason map or a list of `{name, usageDescription}`.
/// AltStore's older `permissions` list (`{type, usageDescription}`) names
/// privacy permissions by type, so it's left out.
fn permissions(app: &Map<String, Value>) -> AppPermissions {
    let Some(Value::Object(permissions)) = app.get("appPermissions") else {
        return AppPermissions::default();
    };
    let entitlements = permissions
        .get("entitlements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| match item {
            Value::String(key) => Some(key.trim().to_string()),
            Value::Object(object) => text(object, "name"),
            _ => None,
        })
        .filter(|key| !key.is_empty())
        .collect();
    let privacy = match permissions.get("privacy") {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, reason)| {
                (
                    key.clone(),
                    reason
                        .as_str()
                        .map(str::trim)
                        .filter(|r| !r.is_empty())
                        .map(String::from),
                )
            })
            .collect(),
        Some(Value::Array(list)) => list
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|item| Some((text(item, "name")?, text(item, "usageDescription"))))
            .collect(),
        _ => Vec::new(),
    };
    AppPermissions {
        entitlements,
        privacy,
    }
}

/// What a link to a source manager asks for
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreLink {
    /// Add the source at this URL
    Source(String),
    /// Download the app at this URL
    Install(String),
}

/// Understand a source manager's link: `altstore://source?url=…`,
/// `sidestore://source?url=…`, `feather://source/…` (or `?url=`), the
/// same with `install`, and `warpsign://` alike. A web link wrapping one
/// (a redirector page, e.g. `…/http-protocol-redirector?r=altstore://…`)
/// is unwrapped. None for anything else.
pub fn parse_store_link(text: &str) -> Option<StoreLink> {
    const SCHEMES: &[&str] = &["altstore", "sidestore", "feather", "warpsign"];
    let text = text.trim();
    let url = url::Url::parse(text).ok()?;
    let scheme = url.scheme().to_ascii_lowercase();
    if scheme == "http" || scheme == "https" {
        // A redirector: a query value that's itself a store link
        return url
            .query_pairs()
            .find_map(|(_, value)| parse_store_link(&value));
    }
    if !SCHEMES.contains(&scheme.as_str()) {
        return None;
    }
    // "altstore://source?url=X": the action is the host. Feather also puts
    // the URL in the path: "feather://source/https://…".
    let action = url.host_str()?.to_ascii_lowercase();
    let target = url
        .query_pairs()
        .find(|(key, _)| key == "url")
        .map(|(_, value)| value.into_owned())
        .or_else(|| {
            let start = text.find("://")? + 3 + action.len();
            let rest = text.get(start..)?.trim_start_matches('/');
            let rest = rest.split(['?', '#']).next().unwrap_or(rest);
            // A URL in a path loses a slash after its scheme ("https:/x")
            let rest = rest.replacen(":/", "://", 1).replacen(":///", "://", 1);
            (!rest.is_empty()).then_some(rest)
        })?;
    let target = target.trim().to_string();
    if !(target.starts_with("https://") || target.starts_with("http://")) {
        return None;
    }
    match action.as_str() {
        "source" => Some(StoreLink::Source(target)),
        "install" => Some(StoreLink::Install(target)),
        _ => None,
    }
}

/// The source URL to add for what the user typed or pasted: a store link
/// (or a web link wrapping one) gives its source, a bare address gets
/// https. None when it isn't a web address.
pub fn resolve_source_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(StoreLink::Source(url)) = parse_store_link(text) {
        return Some(url);
    }
    let text = if text.contains("://") {
        text.to_string()
    } else {
        format!("https://{text}")
    };
    let url = url::Url::parse(&text).ok()?;
    (matches!(url.scheme(), "https" | "http") && url.host_str().is_some()).then_some(text)
}

/// A version entry, or (with `date_key`) the legacy fields on an app
fn parse_version(item: &Map<String, Value>, date_key: Option<&str>) -> Option<AppVersion> {
    let notes_key = if date_key.is_some() {
        "versionDescription"
    } else {
        "localizedDescription"
    };
    Some(AppVersion {
        version: text(item, "version")?,
        download_url: text(item, "downloadURL")?,
        build: text(item, "buildVersion"),
        date: text(item, date_key.unwrap_or("date")),
        notes: text(item, notes_key),
        size: item.get("size").and_then(number),
        min_os: text(item, "minOSVersion"),
        max_os: text(item, "maxOSVersion"),
    })
}

/// `screenshotURLs`, or the newer `screenshots`: a list of URLs or
/// `{imageURL}` objects, optionally split into `iphone` and `ipad`
fn screenshots(app: &Map<String, Value>) -> Vec<String> {
    fn urls(value: Option<&Value>) -> Vec<String> {
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| match item {
                Value::String(url) => Some(url.clone()),
                Value::Object(object) => text(object, "imageURL"),
                _ => None,
            })
            .filter(|url| !url.is_empty())
            .collect()
    }
    match app.get("screenshots") {
        Some(Value::Object(devices)) => {
            let iphone = urls(devices.get("iphone"));
            if iphone.is_empty() {
                urls(devices.get("ipad"))
            } else {
                iphone
            }
        }
        Some(list @ Value::Array(_)) => urls(Some(list)),
        _ => urls(app.get("screenshotURLs")),
    }
}

/// A non-empty string, also accepting numbers ("version": 1.2)
fn text(object: &Map<String, Value>, key: &str) -> Option<String> {
    match object.get(key)? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect()
}

/// A size: an integer, a float, or a string of either
fn number(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Value::String(s) => s.trim().parse::<f64>().ok().map(|f| f.max(0.0) as u64),
        _ => None,
    }
}

/// "5C7AEA" or "#5c7aea" to "#5C7AEA"
fn hex_color(color: String) -> String {
    format!("#{}", color.trim_start_matches('#').to_ascii_uppercase())
}

/// Whether a source's version is newer than the one in the library, by
/// version, then build when the versions are the same. Lenient, as sources
/// are: `v1.2`, `1.10` > `1.9`, a pre-release (`1.2-beta`) before its
/// release, missing parts as 0. Different but incomparable strings aren't
/// newer, so nothing updates on a guess.
pub fn is_newer_version(
    candidate: &str,
    candidate_build: Option<&str>,
    current: &str,
    current_build: Option<&str>,
) -> bool {
    use std::cmp::Ordering;
    match compare_versions(candidate, current) {
        Some(Ordering::Greater) => true,
        Some(Ordering::Equal) => match (candidate_build, current_build) {
            (Some(new), Some(old)) => compare_versions(new, old) == Some(Ordering::Greater),
            _ => false,
        },
        _ => false,
    }
}

/// Whether a version is a pre-release: letters after the numbers
/// (`1.2-beta`, `2.0b3`, `3.1 RC1`). An unreadable version isn't one.
pub fn is_pre_release(version: &str) -> bool {
    version_parts(version)
        .and_then(|(_, pre)| pre)
        .is_some_and(|pre| pre.chars().any(|c| c.is_ascii_alphabetic()))
}

fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let (a_numbers, a_pre) = version_parts(a)?;
    let (b_numbers, b_pre) = version_parts(b)?;
    let length = a_numbers.len().max(b_numbers.len());
    for index in 0..length {
        let x = a_numbers.get(index).copied().unwrap_or(0);
        let y = b_numbers.get(index).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => {}
            other => return Some(other),
        }
    }
    // Same numbers: a release is newer than its pre-releases
    Some(match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(&y),
    })
}

/// "v1.2.3-beta.2" → ([1, 2, 3], Some("beta.2")); None without a number
fn version_parts(text: &str) -> Option<(Vec<u64>, Option<String>)> {
    let text = text.trim();
    let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (numbers, rest) = text.split_at(end);
    let numbers: Vec<u64> = numbers
        .split('.')
        .filter(|part| !part.is_empty())
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    if numbers.is_empty() {
        return None;
    }
    let pre = rest.trim_start_matches(['-', '+', ' ', '_']);
    Some((numbers, (!pre.is_empty()).then(|| pre.to_ascii_lowercase())))
}

#[cfg(test)]
mod version_tests {
    use super::is_newer_version as newer;

    #[test]
    fn compares_versions_as_numbers() {
        assert!(newer("1.10", None, "1.9", None));
        assert!(newer("2.0", None, "1.99.9", None));
        assert!(newer("1.2.1", None, "1.2", None));
        assert!(!newer("1.2", None, "1.2.0", None));
        assert!(!newer("1.9", None, "1.10", None));
        assert!(newer("v21.42.1", None, "21.41.5", None));
    }

    #[test]
    fn releases_beat_their_pre_releases() {
        assert!(newer("1.2", None, "1.2-beta", None));
        assert!(!newer("1.2-beta", None, "1.2", None));
        assert!(newer("1.2-beta.2", None, "1.2-beta.1", None));
        assert!(newer("1.3-beta", None, "1.2", None));
    }

    #[test]
    fn builds_break_ties() {
        assert!(newer("1.2", Some("45"), "1.2", Some("44")));
        assert!(!newer("1.2", Some("44"), "1.2", Some("45")));
        assert!(!newer("1.2", Some("45"), "1.2", None));
        assert!(!newer("1.2", None, "1.2", None));
    }

    #[test]
    fn unwraps_store_links() {
        use super::{StoreLink, parse_store_link, resolve_source_url};
        let source = "https://raw.githubusercontent.com/NeoFreeBird/app/refs/heads/v7/resources/AltSource.json";
        for link in [
            format!(
                "https://intradeus.github.io/http-protocol-redirector?r=altstore://source?url={source}"
            ),
            format!(
                "https://intradeus.github.io/http-protocol-redirector?r=sidestore://source?url={source}"
            ),
            format!(
                "https://intradeus.github.io/http-protocol-redirector?r=feather://source/{source}"
            ),
            format!("altstore://source?url={source}"),
            format!(
                "sidestore://source?url={}",
                url::form_urlencoded::byte_serialize(source.as_bytes()).collect::<String>()
            ),
            format!("feather://source/{source}"),
            format!("warpsign://source?url={source}"),
        ] {
            assert_eq!(
                parse_store_link(&link),
                Some(StoreLink::Source(source.into())),
                "{link}"
            );
            assert_eq!(resolve_source_url(&link).as_deref(), Some(source), "{link}");
        }
        assert_eq!(
            parse_store_link("altstore://install?url=https://example.com/app.ipa"),
            Some(StoreLink::Install("https://example.com/app.ipa".into()))
        );
    }

    #[test]
    fn resolves_plain_sources() {
        use super::{parse_store_link, resolve_source_url};
        assert_eq!(
            resolve_source_url(" repo.example.com/apps.json ").as_deref(),
            Some("https://repo.example.com/apps.json")
        );
        assert_eq!(
            resolve_source_url("https://example.com/a.json?v=2").as_deref(),
            Some("https://example.com/a.json?v=2")
        );
        assert_eq!(parse_store_link("https://example.com/a.json?v=2"), None);
        assert_eq!(
            parse_store_link("altstore://source?url=javascript:alert(1)"),
            None
        );
        assert_eq!(resolve_source_url("not a url"), None);
        assert_eq!(resolve_source_url(""), None);
    }

    #[test]
    fn finds_pre_releases() {
        use super::is_pre_release as pre;
        assert!(pre("1.2-beta"));
        assert!(pre("2.0b3"));
        assert!(pre("v3.1 RC1"));
        assert!(!pre("1.2"));
        assert!(!pre("v1.2.3"));
        assert!(!pre("1.2 (45)"));
        assert!(!pre("nightly"));
    }

    #[test]
    fn never_guesses() {
        assert!(!newer("latest", None, "1.0", None));
        assert!(!newer("1.0", None, "nightly", None));
        assert!(!newer("", None, "", None));
    }
}

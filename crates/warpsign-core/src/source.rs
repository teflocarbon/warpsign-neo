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
    })
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

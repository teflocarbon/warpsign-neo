//! Parsing AltStore-format app sources.

use warpsign_core::source::{SourceError, parse_source};

fn example() -> Vec<u8> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sources/altstore.json");
    std::fs::read(path).unwrap()
}

#[test]
fn parses_source_details() {
    let source = parse_source(&example()).unwrap();
    assert_eq!(source.name, "Example Source");
    assert_eq!(source.identifier.as_deref(), Some("com.example.source"));
    assert_eq!(source.tint_color.as_deref(), Some("#5C7AEA"));
    assert_eq!(source.featured_apps, ["com.example.modern"]);
    assert_eq!(source.news.len(), 1);
    assert_eq!(source.news[0].app_id.as_deref(), Some("com.example.modern"));
}

#[test]
fn parses_modern_apps() {
    let source = parse_source(&example()).unwrap();
    let app = &source.apps[0];
    assert_eq!(app.bundle_id, "com.example.modern");
    assert_eq!(app.developer_name.as_deref(), Some("Example Dev"));
    assert_eq!(app.tint_color.as_deref(), Some("#FF0000"));
    // iPhone screenshots preferred, in both URL and object form
    assert_eq!(
        app.screenshot_urls,
        ["https://example.com/1.png", "https://example.com/2.png"]
    );
    assert_eq!(app.versions.len(), 2);
    let latest = app.latest();
    assert_eq!(latest.version, "2.0");
    assert_eq!(latest.build.as_deref(), Some("20"));
    assert_eq!(latest.notes.as_deref(), Some("New things."));
    assert_eq!(latest.size, Some(12_345_678));
    assert_eq!(latest.min_os.as_deref(), Some("16.0"));
    assert_eq!(app.versions[1].size, Some(12_000_000));
}

#[test]
fn parses_legacy_apps() {
    let source = parse_source(&example()).unwrap();
    let app = &source.apps[1];
    assert_eq!(app.bundle_id, "com.example.legacy");
    assert_eq!(app.screenshot_urls, ["https://example.com/legacy1.png"]);
    let latest = app.latest();
    assert_eq!(latest.version, "1.5");
    assert_eq!(latest.date.as_deref(), Some("2021-05-01"));
    assert_eq!(latest.notes.as_deref(), Some("Bug fixes."));
    assert_eq!(latest.download_url, "https://example.com/Legacy.ipa");
    assert_eq!(latest.size, Some(4_567_890));
}

#[test]
fn skips_broken_apps_with_warnings() {
    let source = parse_source(&example()).unwrap();
    assert_eq!(source.apps.len(), 2);
    assert_eq!(source.warnings.len(), 2);
    assert!(
        source.warnings[0].contains("\"Broken\""),
        "{:?}",
        source.warnings
    );
    assert!(source.warnings[1].contains("#4"), "{:?}", source.warnings);
}

#[test]
fn rejects_non_sources() {
    assert!(matches!(parse_source(b"<html>"), Err(SourceError::Json(_))));
    assert!(matches!(parse_source(b"[]"), Err(SourceError::Format(_))));
    assert!(matches!(
        parse_source(br#"{"name": "x"}"#),
        Err(SourceError::Format(_))
    ));
    let empty = parse_source(br#"{"apps": []}"#).unwrap();
    assert_eq!(empty.name, "Untitled source");
}

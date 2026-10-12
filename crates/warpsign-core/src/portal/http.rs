//! HTTP for the portal client.
//!
//! The platform supplies a [`Transport`] that sends one request and returns
//! the response as-is: URLSession in the iOS app, a plain HTTP client in the
//! CLI. Cookies and redirects are handled here, so the session works the
//! same everywhere and can be saved (see [`SessionState`]) and restored.

use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Head => "HEAD",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new(Method::Get, url)
    }

    pub fn post(url: impl Into<String>) -> Self {
        Self::new(Method::Post, url)
    }

    pub fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    pub fn headers(mut self, headers: &[(&str, &str)]) -> Self {
        for (name, value) in headers {
            self.headers.push((name.to_string(), value.to_string()));
        }
        self
    }

    /// Append URL query parameters, encoded like Python's `urlencode`
    pub fn query(mut self, params: &[(&str, &str)]) -> Self {
        let separator = if self.url.contains('?') { '&' } else { '?' };
        self.url = format!("{}{separator}{}", self.url, form_encode(params));
        self
    }

    /// A JSON body. Doesn't set Content-Type: the portal wants different
    /// ones on different endpoints.
    pub fn json(mut self, body: &serde_json::Value) -> Self {
        self.body = serde_json::to_vec(body).expect("JSON values always serialize");
        self
    }

    /// An `application/x-www-form-urlencoded` body
    pub fn form(mut self, params: &[(&str, &str)]) -> Self {
        self.body = form_encode(params).into_bytes();
        self
    }

    pub fn raw_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    pub fn header_value(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }

    pub fn json_body(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub status: u16,
    /// Every header line as received; `Set-Cookie` may repeat
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }

    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct TransportError(pub String);

/// Sends a single HTTP request.
///
/// Implementations must not follow redirects and must not store or send
/// cookies themselves; [`HttpClient`] does both.
pub trait Transport: Send + Sync {
    fn send(&self, request: &Request) -> Result<Response, TransportError>;
}

/// A cookie as stored in the session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lowercase, without a leading dot
    pub domain: String,
    /// Sent only to `domain` itself, not its subdomains
    pub host_only: bool,
    pub path: String,
    /// Seconds since 1970; `None` for session cookies (kept anyway, like
    /// Python warpsign's cookie jar)
    pub expires: Option<u64>,
}

/// Everything needed to resume a signed-in session. Secret: keep it in the
/// Keychain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub cookies: Vec<Cookie>,
    /// `X-Apple-ID-Session-Id`, from sign-in
    pub apple_session_id: Option<String>,
    pub scnt: Option<String>,
    /// The portal's anti-CSRF tokens, required on changes
    pub csrf: Option<String>,
    pub csrf_ts: Option<String>,
}

impl SessionState {
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|cookie| cookie.name == name)
            .map(|cookie| cookie.value.as_str())
    }

    /// Only what a new sign-in should start from: Apple's two-factor trust
    /// cookies (`DES…`, from `2sv/trust`, kept about a month). Sent at
    /// sign-in, they let Apple skip asking for a code, as a browser that
    /// was told to trust itself does.
    pub fn trust(&self) -> SessionState {
        let cookies = self
            .cookies
            .iter()
            .filter(|cookie| cookie.name.starts_with("DES"))
            .cloned()
            .map(|mut cookie| {
                // Sessions saved before quotes were kept lost them
                if !cookie.value.starts_with('"') {
                    cookie.value = format!("\"{}\"", cookie.value);
                }
                cookie
            })
            .collect();
        SessionState {
            cookies,
            ..SessionState::default()
        }
    }

    fn store_cookie(&mut self, cookie: Cookie, now: u64) {
        self.cookies.retain(|existing| {
            !(existing.name == cookie.name
                && existing.domain == cookie.domain
                && existing.path == cookie.path)
        });
        if cookie.expires.is_none_or(|expires| expires > now) {
            self.cookies.push(cookie);
        }
    }

    fn cookie_header(&self, host: &str, path: &str, now: u64) -> Option<String> {
        let mut matching: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|cookie| cookie.expires.is_none_or(|expires| expires > now))
            .filter(|cookie| {
                if cookie.host_only {
                    host == cookie.domain
                } else {
                    host == cookie.domain || host.ends_with(&format!(".{}", cookie.domain))
                }
            })
            .filter(|cookie| path_matches(path, &cookie.path))
            .collect();
        if matching.is_empty() {
            return None;
        }
        // More specific paths first, as browsers do
        matching.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));
        Some(
            matching
                .iter()
                .map(|cookie| format!("{}={}", cookie.name, cookie.value))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }
}

fn path_matches(request_path: &str, cookie_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

/// Sends requests through a [`Transport`], keeping the session's cookies and
/// following redirects
pub struct HttpClient {
    transport: Arc<dyn Transport>,
    state: Mutex<SessionState>,
}

const MAX_REDIRECTS: usize = 10;

impl HttpClient {
    pub fn new(transport: Arc<dyn Transport>) -> Self {
        Self::with_state(transport, SessionState::default())
    }

    pub fn with_state(transport: Arc<dyn Transport>, state: SessionState) -> Self {
        Self {
            transport,
            state: Mutex::new(state),
        }
    }

    pub fn state(&self) -> SessionState {
        self.lock().clone()
    }

    pub fn update_state(&self, update: impl FnOnce(&mut SessionState)) {
        update(&mut self.lock());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SessionState> {
        // A panic while holding the lock can't leave the state half-written
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Send without the session's cookies and without following
    /// redirects, as a fresh browser would
    pub fn send_bare(&self, request: &Request) -> Result<Response, TransportError> {
        self.transport.send(request)
    }

    pub fn send(&self, request: Request) -> Result<Response, TransportError> {
        let mut request = request;
        for _ in 0..=MAX_REDIRECTS {
            let url = Url::parse(&request.url)
                .ok_or_else(|| TransportError(format!("invalid URL: {}", request.url)))?;
            let now = now();
            let mut outgoing = request.clone();
            outgoing
                .headers
                .retain(|(name, _)| !name.eq_ignore_ascii_case("cookie"));
            if let Some(cookies) = self.lock().cookie_header(&url.host, &url.path, now) {
                outgoing.headers.push(("Cookie".into(), cookies));
            }

            let response = self.transport.send(&outgoing)?;
            self.remember(&url, &response, now);

            let location = response.header("Location").map(String::from);
            match (response.status, location) {
                (301 | 302 | 303 | 307 | 308, Some(location)) => {
                    let target = url.join(&location);
                    request = if matches!(response.status, 307 | 308) {
                        Request {
                            url: target,
                            ..request
                        }
                    } else {
                        let mut redirected = Request::get(target);
                        redirected.headers = request
                            .headers
                            .into_iter()
                            .filter(|(name, _)| !name.eq_ignore_ascii_case("content-type"))
                            .collect();
                        redirected
                    };
                }
                _ => return Ok(response),
            }
        }
        Err(TransportError("too many redirects".into()))
    }

    fn remember(&self, url: &Url, response: &Response, now: u64) {
        let mut state = self.lock();
        for (name, value) in &response.headers {
            if !name.eq_ignore_ascii_case("set-cookie") {
                continue;
            }
            for line in split_set_cookie(value) {
                if let Some(cookie) = parse_set_cookie(line, url, now) {
                    state.store_cookie(cookie, now);
                }
            }
        }
        // The portal hands out fresh anti-CSRF tokens in response headers
        if let (Some(csrf), Some(csrf_ts)) = (response.header("csrf"), response.header("csrf_ts")) {
            state.csrf = Some(csrf.to_string());
            state.csrf_ts = Some(csrf_ts.to_string());
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Just enough URL handling for cookies and redirects
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Url {
    pub scheme: String,
    pub host: String,
    /// Without the query
    pub path: String,
}

impl Url {
    pub(crate) fn parse(url: &str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let host = authority
            .rsplit('@')
            .next()?
            .split(':')
            .next()?
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }
        let after = &rest[end..];
        let path_end = after.find(['?', '#']).unwrap_or(after.len());
        let path = match &after[..path_end] {
            "" => "/".to_string(),
            path => path.to_string(),
        };
        Some(Self {
            scheme: scheme.to_ascii_lowercase(),
            host,
            path,
        })
    }

    fn join(&self, location: &str) -> String {
        if location.contains("://") {
            location.to_string()
        } else if let Some(rest) = location.strip_prefix("//") {
            format!("{}://{rest}", self.scheme)
        } else if location.starts_with('/') {
            format!("{}://{}{location}", self.scheme, self.host)
        } else {
            let directory = &self.path[..=self.path.rfind('/').unwrap_or(0)];
            format!("{}://{}{directory}{location}", self.scheme, self.host)
        }
    }
}

/// Split a `Set-Cookie` value that a platform HTTP stack joined with commas.
/// Commas inside `Expires=Wed, 21 Oct ...` aren't followed by `name=`.
fn split_set_cookie(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    for (index, _) in value.match_indices(',') {
        let next = value[index + 1..].trim_start();
        let token_end = next.find(['=', ';', ',', ' ']).unwrap_or(next.len());
        if token_end > 0 && next[token_end..].starts_with('=') {
            parts.push(value[start..index].trim());
            start = index + 1;
        }
    }
    parts.push(value[start..].trim());
    parts.retain(|part| !part.is_empty());
    parts
}

fn parse_set_cookie(line: &str, url: &Url, now: u64) -> Option<Cookie> {
    let mut attributes = line.split(';');
    let (name, value) = attributes.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let mut cookie = Cookie {
        name: name.to_string(),
        // Verbatim, quotes included, as browsers and Python's cookiejar
        // keep it: Apple's two-factor trust cookie (DES…) is quoted, and is
        // only accepted back with its quotes (fastlane re-adds them too)
        value: value.trim().to_string(),
        domain: url.host.clone(),
        host_only: true,
        path: default_path(&url.path),
        expires: None,
    };
    let mut max_age = None;
    for attribute in attributes {
        let (key, value) = attribute.split_once('=').unwrap_or((attribute, ""));
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "domain" if !value.is_empty() => {
                let domain = value.trim_start_matches('.').to_ascii_lowercase();
                // Only the host itself or a parent domain of it
                if url.host != domain && !url.host.ends_with(&format!(".{domain}")) {
                    return None;
                }
                cookie.domain = domain;
                cookie.host_only = false;
            }
            "path" if value.starts_with('/') => cookie.path = value.to_string(),
            "max-age" => max_age = value.parse::<i64>().ok(),
            "expires" if cookie.expires.is_none() => cookie.expires = parse_http_date(value),
            _ => {}
        }
    }
    if let Some(max_age) = max_age {
        // Max-Age wins over Expires
        cookie.expires = Some(if max_age <= 0 {
            0
        } else {
            now.saturating_add(max_age as u64)
        });
    }
    Some(cookie)
}

fn default_path(request_path: &str) -> String {
    match request_path.rfind('/') {
        Some(0) | None => "/".into(),
        Some(index) => request_path[..index].to_string(),
    }
}

/// "Wed, 21 Oct 2015 07:28:00 GMT", also with dashes ("21-Oct-2015") and
/// two-digit years, as seconds since 1970
pub(crate) fn parse_http_date(value: &str) -> Option<u64> {
    let value = value.split_once(',').map_or(value, |(_, rest)| rest);
    let fields: Vec<&str> = value
        .split([' ', '-'])
        .filter(|field| !field.is_empty())
        .collect();
    let [day, month, year, time, ..] = fields.as_slice() else {
        return None;
    };
    let day: u32 = day.parse().ok()?;
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|name| month.to_ascii_lowercase().starts_with(name))? as u32
        + 1;
    let mut year: i64 = year.parse().ok()?;
    if year < 70 {
        year += 2000;
    } else if year < 100 {
        year += 1900;
    }
    let mut clock = time.split(':').map(|part| part.parse::<u64>().ok());
    let (hours, minutes, seconds) = (clock.next()??, clock.next()??, clock.next()??);
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return Some(0);
    }
    Some(days as u64 * 86_400 + hours * 3600 + minutes * 60 + seconds)
}

/// Days since 1970-01-01, from Howard Hinnant's `days_from_civil`
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `application/x-www-form-urlencoded`, matching Python's `urlencode`
/// (`quote_plus`: letters, digits and `_.-~` stay, spaces become `+`)
pub fn form_encode(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(key, value)| format!("{}={}", quote_plus(key), quote_plus(value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn quote_plus(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn parses_urls() {
        let parsed = url("https://Developer.Apple.com/services-account/v1/certificates?x=1");
        assert_eq!(parsed.host, "developer.apple.com");
        assert_eq!(parsed.path, "/services-account/v1/certificates");
        assert_eq!(url("https://idmsa.apple.com").path, "/");
        assert_eq!(url("https://a.com:443/x").host, "a.com");
        assert!(Url::parse("not a url").is_none());
    }

    #[test]
    fn joins_redirect_locations() {
        let base = url("https://developer.apple.com/account/resources");
        assert_eq!(
            base.join("https://idmsa.apple.com/x"),
            "https://idmsa.apple.com/x"
        );
        assert_eq!(base.join("/login"), "https://developer.apple.com/login");
        assert_eq!(base.join("//cdn.apple.com/a"), "https://cdn.apple.com/a");
        assert_eq!(
            base.join("list"),
            "https://developer.apple.com/account/list"
        );
    }

    #[test]
    fn encodes_forms_like_python() {
        // urllib.parse.urlencode({"a": "x y", "b": "s=1&t=[2],~"})
        assert_eq!(
            form_encode(&[("a", "x y"), ("b", "s=1&t=[2],~")]),
            "a=x+y&b=s%3D1%26t%3D%5B2%5D%2C~"
        );
    }

    #[test]
    fn parses_http_dates() {
        assert_eq!(
            parse_http_date("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(1_445_412_480)
        );
        assert_eq!(parse_http_date("Thu, 01-Jan-1970 00:00:10 GMT"), Some(10));
        assert_eq!(
            parse_http_date("Sat, 01-Jan-00 00:00:00 GMT"),
            Some(946_684_800)
        );
        assert_eq!(parse_http_date("garbage"), None);
    }

    #[test]
    fn splits_joined_set_cookie_headers() {
        let joined =
            "a=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT; Path=/, b=2; Domain=.apple.com,c=3";
        assert_eq!(
            split_set_cookie(joined),
            [
                "a=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT; Path=/",
                "b=2; Domain=.apple.com",
                "c=3"
            ]
        );
    }

    #[test]
    fn scopes_cookies_by_domain_and_path() {
        let now = 1_000;
        let mut state = SessionState::default();
        let idmsa = url("https://idmsa.apple.com/appleauth/auth/signin");
        for line in [
            "myacinfo=secret; Domain=.apple.com; Path=/; Secure; HttpOnly",
            "aasp=host; Path=/",
            "deep=1; Path=/appleauth/auth",
            "evil=1; Domain=example.com",
            "gone=1; Max-Age=0",
            "old=1; Expires=Thu, 01 Jan 1970 00:00:00 GMT",
            "later=1; Max-Age=60; Path=/",
        ] {
            if let Some(cookie) = parse_set_cookie(line, &idmsa, now) {
                state.store_cookie(cookie, now);
            }
        }
        assert_eq!(
            state
                .cookie_header("idmsa.apple.com", "/appleauth/auth/2sv/trust", now)
                .as_deref(),
            Some("deep=1; myacinfo=secret; aasp=host; later=1")
        );
        assert_eq!(
            state
                .cookie_header("developer.apple.com", "/account", now)
                .as_deref(),
            Some("myacinfo=secret")
        );
        assert_eq!(
            state
                .cookie_header("idmsa.apple.com", "/appleauth/authx", now)
                .as_deref(),
            Some("myacinfo=secret; aasp=host; later=1")
        );
        assert_eq!(state.cookie_header("example.com", "/", now), None);
        // Expired since
        assert!(
            !state
                .cookie_header("idmsa.apple.com", "/", now + 61)
                .unwrap()
                .contains("later")
        );

        // Replaced by name, domain and path; deleted by an expired one
        let replace = parse_set_cookie("aasp=new; Path=/", &idmsa, now).unwrap();
        state.store_cookie(replace, now);
        assert_eq!(state.cookie("aasp"), Some("new"));
        let delete = parse_set_cookie("aasp=; Max-Age=0; Path=/", &idmsa, now).unwrap();
        state.store_cookie(delete, now);
        assert_eq!(state.cookie("aasp"), None);
    }

    #[test]
    fn keeps_quoted_values_as_they_are() {
        let now = 1_000;
        let mut state = SessionState::default();
        let idmsa = url("https://idmsa.apple.com/appleauth/auth/2sv/trust");
        let line = "DES58b0eba5=\"HSARMTKN/x+y==SRVT\"; Max-Age=2592000; Path=/; Domain=.idmsa.apple.com; Secure; HttpOnly";
        state.store_cookie(parse_set_cookie(line, &idmsa, now).unwrap(), now);
        assert_eq!(
            state
                .cookie_header("idmsa.apple.com", "/appleauth/auth/signin/complete", now)
                .as_deref(),
            Some("DES58b0eba5=\"HSARMTKN/x+y==SRVT\"")
        );
    }

    #[test]
    fn trust_keeps_only_the_two_factor_cookies() {
        let now = 1_000;
        let mut state = SessionState {
            scnt: Some("scnt".into()),
            csrf: Some("csrf".into()),
            ..SessionState::default()
        };
        let idmsa = url("https://idmsa.apple.com/appleauth/auth/2sv/trust");
        for line in [
            "myacinfo=secret; Domain=.apple.com; Path=/",
            "DESa=\"quoted\"; Max-Age=2592000; Path=/",
            "DESb=unquoted; Max-Age=2592000; Path=/",
        ] {
            state.store_cookie(parse_set_cookie(line, &idmsa, now).unwrap(), now);
        }
        let trust = state.trust();
        assert_eq!(trust.scnt, None);
        assert_eq!(trust.csrf, None);
        assert_eq!(
            trust
                .cookie_header("idmsa.apple.com", "/appleauth/auth/signin/init", now)
                .as_deref(),
            Some("DESa=\"quoted\"; DESb=\"unquoted\"")
        );
    }
}

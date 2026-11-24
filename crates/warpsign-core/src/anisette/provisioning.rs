//! Provisioning an ADI identity with Apple (GrandSlam's
//! midStartProvisioning and midFinishProvisioning). No Apple ID is
//! involved: it registers a machine, as a new Mac or iPhone does.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use plist::{Dictionary, Value};

use super::adi::Adi;
use super::{AnisetteError, AnisetteState, client_time};
use crate::portal::http::{Request, Transport};

const LOOKUP: &str = "https://gsa.apple.com/grandslam/GsService2/lookup";

fn headers(request: Request, state: &AnisetteState) -> Request {
    request
        .header("User-Agent", "akd/1.0 CFNetwork/1404.0.5 Darwin/22.3.0")
        // AuthKit sends plists with this type
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("X-Mme-Device-Id", &state.device_id)
        .header("X-MMe-Client-Info", &state.client_info)
        .header("X-Apple-I-MD-LU", &state.local_user)
        .header("X-Apple-Client-App-Name", "Setup")
        .header("X-Apple-I-Client-Time", client_time())
}

fn send(transport: &dyn Transport, request: Request) -> Result<Dictionary, AnisetteError> {
    let url = request.url.clone();
    let response = transport
        .send(&request)
        .map_err(|e| AnisetteError::Http(e.to_string()))?;
    let plist = Value::from_reader(std::io::Cursor::new(&response.body))
        .ok()
        .and_then(Value::into_dictionary);
    if !response.is_success() {
        // Apple's error, if it sent one in the usual place
        let status = plist
            .as_ref()
            .and_then(|plist| response_of(plist, "").err())
            .map(|e| format!(": {e}"))
            .unwrap_or_default();
        return Err(AnisetteError::Apple(format!(
            "{url} answered HTTP {}{status}",
            response.status
        )));
    }
    plist.ok_or_else(|| AnisetteError::Apple(format!("{url} didn't answer with a plist")))
}

fn body(request: Dictionary) -> Vec<u8> {
    let mut root = Dictionary::new();
    root.insert("Header".into(), Value::Dictionary(Dictionary::new()));
    root.insert("Request".into(), Value::Dictionary(request));
    let mut bytes = Vec::new();
    plist::to_writer_xml(&mut bytes, &Value::Dictionary(root)).expect("writing to memory");
    bytes
}

/// The `Response` dictionary, or Apple's error from its `Status`
fn response_of(plist: &Dictionary, step: &str) -> Result<Dictionary, AnisetteError> {
    let response = plist
        .get("Response")
        .and_then(Value::as_dictionary)
        .ok_or_else(|| AnisetteError::Apple(format!("{step}: no Response")))?;
    if let Some(status) = response.get("Status").and_then(Value::as_dictionary) {
        let code = status
            .get("ec")
            .and_then(Value::as_signed_integer)
            .unwrap_or(0);
        if code != 0 {
            let message = status
                .get("em")
                .and_then(Value::as_string)
                .unwrap_or("no message");
            return Err(AnisetteError::Apple(format!(
                "{step}: Apple error {code} ({message})"
            )));
        }
    }
    Ok(response.clone())
}

fn data(response: &Dictionary, key: &str, step: &str) -> Result<Vec<u8>, AnisetteError> {
    let text = response
        .get(key)
        .and_then(Value::as_string)
        .ok_or_else(|| AnisetteError::Apple(format!("{step}: no {key}")))?;
    STANDARD
        .decode(text)
        .map_err(|_| AnisetteError::Apple(format!("{step}: {key} isn't base64")))
}

/// Provision `adi` with Apple. Updates `state` with the routing info.
pub fn provision(
    transport: &dyn Transport,
    adi: &mut Adi,
    state: &mut AnisetteState,
) -> Result<(), AnisetteError> {
    let lookup = send(transport, headers(Request::get(LOOKUP), state))?;
    let urls = lookup
        .get("urls")
        .and_then(Value::as_dictionary)
        .ok_or_else(|| AnisetteError::Apple("lookup: no urls".into()))?;
    let url = |name: &str| {
        urls.get(name)
            .and_then(Value::as_string)
            .map(str::to_string)
            .ok_or_else(|| AnisetteError::Apple(format!("lookup: no {name}")))
    };
    let start_url = url("midStartProvisioning")?;
    let finish_url = url("midFinishProvisioning")?;

    let start = send(
        transport,
        headers(Request::post(start_url), state).raw_body(body(Dictionary::new())),
    )?;
    let spim = data(&response_of(&start, "start")?, "spim", "start")?;

    let started = adi.start_provisioning(&spim)?;
    let finished = (|| {
        let mut request = Dictionary::new();
        request.insert("cpim".into(), Value::String(STANDARD.encode(&started.cpim)));
        let finish = send(
            transport,
            headers(Request::post(finish_url), state).raw_body(body(request)),
        )?;
        let finish = response_of(&finish, "finish")?;
        let ptm = data(&finish, "ptm", "finish")?;
        let tk = data(&finish, "tk", "finish")?;
        let routing = finish.get("X-Apple-I-MD-RINFO").and_then(|v| {
            v.as_string()
                .map(str::to_string)
                .or_else(|| v.as_unsigned_integer().map(|n| n.to_string()))
        });
        adi.end_provisioning(started.session, &ptm, &tk)?;
        Ok::<_, AnisetteError>(routing)
    })();
    match finished {
        Ok(routing) => {
            if let Some(routing) = routing {
                state.routing_info = routing;
            }
            Ok(())
        }
        Err(e) => {
            let _ = adi.destroy_provisioning(started.session);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_apple_plists() {
        let mut request = Dictionary::new();
        request.insert("cpim".into(), Value::String("AAAA".into()));
        let text = String::from_utf8(body(request)).unwrap();
        assert!(text.contains("<key>Header</key>"));
        assert!(text.contains("<key>cpim</key>"));
        assert!(text.contains("<string>AAAA</string>"));
    }

    #[test]
    fn apple_errors_are_reported() {
        let mut status = Dictionary::new();
        status.insert("ec".into(), Value::Integer((-5003).into()));
        status.insert("em".into(), Value::String("nope".into()));
        let mut inner = Dictionary::new();
        inner.insert("Status".into(), Value::Dictionary(status));
        let mut plist = Dictionary::new();
        plist.insert("Response".into(), Value::Dictionary(inner));
        let error = response_of(&plist, "start").unwrap_err().to_string();
        assert!(error.contains("-5003") && error.contains("nope"), "{error}");
    }
}

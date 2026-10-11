//! Developer Portal requests for one team, ported from Python warpsign's
//! `DeveloperPortalAPI`: same endpoints, headers and bodies, except that
//! failures are errors with Apple's message instead of `None`/`[]`.
//!
//! Headers a platform HTTP stack owns (`Host`, `Connection`,
//! `Accept-Encoding`) are left to the transport.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

use super::PortalError;
use super::http::{HttpClient, Method, Request, Response};
use crate::ProfileType;
use crate::identity::CertificateKind;

const SERVICES: &str = "https://developer.apple.com/services-account";
const V1: &str = "https://developer.apple.com/services-account/v1";
const LEGACY: &str = "https://developer.apple.com/services-account/QH65B2/account";

/// Capabilities that need settings Apple's API doesn't explain how to choose:
/// Python warpsign's `CAPABILITY_SETTINGS`, as-is
pub fn capability_settings(capability: &str) -> Option<Value> {
    let settings = match capability {
        "ENABLED_FOR_MAC" => json!([
            {"key": "ENABLED_FOR_MAC_APP_SETUP", "options": [{"key": "USE_IOS_APPID"}]}
        ]),
        "PUSH_NOTIFICATIONS" => json!([{
            "key": "PUSH_NOTIFICATION_FEATURES",
            "options": [{"key": "PUSH_NOTIFICATION_FEATURE_BROADCAST"}],
        }]),
        "APPLE_ID_AUTH" => json!([{
            "key": "APPLE_ID_AUTH_APP_CONSENT",
            "options": [{"key": "PRIMARY_APP_CONSENT"}],
        }]),
        "DATA_PROTECTION" => json!([{
            "key": "DATA_PROTECTION_PERMISSION_LEVEL",
            "options": [{"key": "COMPLETE_PROTECTION"}],
        }]),
        "ICLOUD" => json!([{"key": "ICLOUD_VERSION", "options": [{"key": "XCODE_6"}]}]),
        _ => return None,
    };
    Some(settings)
}

/// Capabilities whose resources are linked as relationships (Python's
/// `RELATIONSHIP_CAPABILITIES`)
fn relationship_type(capability: &str) -> Option<&'static str> {
    match capability {
        "ICLOUD" => Some("cloudContainers"),
        "APP_GROUPS" => Some("appGroups"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub team_id: String,
    pub name: String,
    pub status: String,
    /// e.g. "INDIVIDUAL", "COMPANY"
    pub kind: String,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalCertificate {
    /// The portal's resource ID
    pub id: String,
    pub serial_number: String,
    pub certificate_type: String,
    pub name: String,
    pub display_name: Option<String>,
    pub owner_id: Option<String>,
    /// e.g. "2027-10-11T12:00:00.000+00:00"
    pub expiration_date: Option<String>,
}

impl PortalCertificate {
    pub fn kind(&self) -> Option<CertificateKind> {
        match self.certificate_type.as_str() {
            "DEVELOPMENT" | "IOS_DEVELOPMENT" => Some(CertificateKind::Development),
            "DISTRIBUTION" | "IOS_DISTRIBUTION" => Some(CertificateKind::Distribution),
            _ => None,
        }
    }
}

/// A registered App ID
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleId {
    pub id: String,
    pub identifier: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGroup {
    pub id: String,
    pub identifier: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudContainer {
    pub id: String,
    pub identifier: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub udid: String,
    pub status: String,
    pub device_class: String,
    pub platform: String,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortalProfile {
    pub id: String,
    pub name: String,
    pub state: String,
    pub platform: String,
    pub type_label: String,
    /// e.g. "2027-10-11T12:00:00.000+00:00"
    pub expiration_date: Option<String>,
}

/// What changed when setting an App ID's capabilities
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityUpdate {
    /// Required by Apple, so enabled whether asked for or not
    pub required: Vec<String>,
    /// Asked for, but left off: Apple wants settings warpsign doesn't know
    /// how to choose
    pub skipped: Vec<String>,
    /// Asked for, but left off when Apple refused them: capabilities teams
    /// must request from Apple (`canRequestFromPortal`), e.g. Family
    /// Controls (Distribution)
    pub not_granted: Vec<String>,
}

/// The portal API for one team
pub struct Portal {
    http: Arc<HttpClient>,
    team_id: String,
    capabilities: Mutex<Option<Value>>,
}

impl Portal {
    pub fn new(http: Arc<HttpClient>, team_id: &str) -> Self {
        Self {
            http,
            team_id: team_id.to_string(),
            capabilities: Mutex::new(None),
        }
    }

    pub fn team_id(&self) -> &str {
        &self.team_id
    }

    pub fn certificates(&self) -> Result<Vec<PortalCertificate>, PortalError> {
        let body = self.list(
            "certificates",
            "limit=1000&sort=displayName",
            "listing certificates",
        )?;
        resources(&body, "listing certificates", certificate_from)
    }

    /// Have the portal issue a certificate for a certificate signing
    /// request. Returns the certificate and its DER contents.
    ///
    /// Not in Python warpsign (it took an existing .p12), so this request
    /// shape follows the App Store Connect API that the portal fronts.
    pub fn create_certificate(
        &self,
        kind: CertificateKind,
        csr_pem: &str,
    ) -> Result<(PortalCertificate, Vec<u8>), PortalError> {
        let operation = "creating a certificate";
        let certificate_type = match kind {
            CertificateKind::Development => "DEVELOPMENT",
            CertificateKind::Distribution => "DISTRIBUTION",
        };
        let request = self
            .write_json(Request::post(format!("{V1}/certificates")))
            .json(&json!({
                "data": {
                    "type": "certificates",
                    "attributes": {
                        "certificateType": certificate_type,
                        "csrContent": csr_pem,
                        "teamId": self.team_id,
                    },
                }
            }));
        let body = self.expect_json(request, operation)?;
        let data = body
            .get("data")
            .ok_or_else(|| PortalError::new(operation, None, "no certificate in the response"))?;
        let certificate = certificate_from(data)
            .ok_or_else(|| PortalError::new(operation, None, "unexpected certificate format"))?;
        let content = match content_from(data) {
            Some(content) => content,
            None => self.certificate_content(&certificate.id)?,
        };
        Ok((certificate, content))
    }

    /// A certificate's DER contents
    pub fn certificate_content(&self, certificate_id: &str) -> Result<Vec<u8>, PortalError> {
        let operation = "downloading a certificate";
        let body = self.expect_json(
            self.read(&format!("{V1}/certificates/{certificate_id}"), ""),
            operation,
        )?;
        body.get("data").and_then(content_from).ok_or_else(|| {
            PortalError::new(operation, None, "no certificate content in the response")
        })
    }

    pub fn bundle_ids(&self) -> Result<Vec<BundleId>, PortalError> {
        let body = self.list(
            "bundleIds",
            "limit=1000&sort=name&filter[platform]=IOS,MACOS",
            "listing App IDs",
        )?;
        resources(&body, "listing App IDs", bundle_id_from)
    }

    /// Register an App ID, or find it if this team already has it
    pub fn register_bundle_id(
        &self,
        identifier: &str,
        name: &str,
    ) -> Result<BundleId, PortalError> {
        let operation = format!("registering App ID {identifier}");
        let request = self
            .write_json(Request::post(format!("{V1}/bundleIds")))
            .json(&json!({
                "data": {
                    "type": "bundleIds",
                    "attributes": {
                        "identifier": identifier,
                        "name": name,
                        "seedId": self.team_id,
                        "teamId": self.team_id,
                    },
                    "relationships": {"bundleIdCapabilities": {"data": []}},
                }
            }));
        let response = self.send(request, &operation)?;
        if response.status == 409 {
            let already_ours = response
                .json()
                .and_then(|body| body.pointer("/errors/0/resultCode")?.as_i64())
                == Some(9400);
            if !already_ours {
                return Err(PortalError::from_response(&operation, &response));
            }
            let body = self.list(
                "bundleIds",
                &format!("filter[identifier]={identifier}"),
                &operation,
            )?;
            return resources(&body, &operation, bundle_id_from)?
                .into_iter()
                .find(|bundle| bundle.identifier == identifier)
                .ok_or_else(|| {
                    PortalError::new(&operation, None, "it exists, but the portal didn't list it")
                });
        }
        let body = json_from(&response, &operation)?;
        body.get("data")
            .and_then(bundle_id_from)
            .ok_or_else(|| PortalError::new(&operation, None, "no App ID in the response"))
    }

    /// Every capability the team can use (`/v1/capabilities`), as the raw
    /// response the entitlements processor reads. Fetched once.
    pub fn capabilities(&self) -> Result<Value, PortalError> {
        let mut cache = self.capabilities.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(capabilities) = cache.as_ref() {
            return Ok(capabilities.clone());
        }
        let request = self
            .read(&format!("{V1}/capabilities"), "filter[platform]=IOS,MACOS")
            .query(&[("filter[capabilityType]", "capability,service")]);
        let body = self.expect_json(request, "listing capabilities")?;
        *cache = Some(body.clone());
        Ok(body)
    }

    /// Set exactly which capabilities an App ID has. `relationships` maps a
    /// capability (APP_GROUPS, ICLOUD) to the resource IDs to link.
    ///
    /// Sends Python warpsign's request. If the portal rejects it and some
    /// requested capabilities need settings warpsign doesn't know, tries
    /// once more without those (docs/design/portal.md).
    pub fn update_capabilities(
        &self,
        bundle: &BundleId,
        enable: &[String],
        relationships: &BTreeMap<String, Vec<String>>,
    ) -> Result<CapabilityUpdate, PortalError> {
        let operation = format!("setting capabilities of {}", bundle.identifier);
        let available = self.capabilities()?;
        let available: Vec<&Value> = available
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();

        let mut update = CapabilityUpdate::default();
        let mut unknown_settings = Vec::new();
        let mut request_only = Vec::new();
        for capability in &available {
            let Some(id) = capability.get("id").and_then(Value::as_str) else {
                continue;
            };
            let attributes = capability.get("attributes");
            let optional = attributes
                .and_then(|a| a.get("optional"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if !optional {
                update.required.push(id.to_string());
            } else if enable.iter().any(|e| e == id) {
                if attributes
                    .and_then(|a| a.get("canRequestFromPortal"))
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    request_only.push(id.to_string());
                }
                let has_settings = attributes
                    .and_then(|a| a.get("settings"))
                    .and_then(Value::as_array)
                    .is_some_and(|settings| !settings.is_empty());
                if has_settings && capability_settings(id).is_none() {
                    unknown_settings.push(id.to_string());
                }
            }
        }

        let first =
            self.patch_capabilities(bundle, &available, enable, &[], relationships, &operation)?;
        if first.is_success() {
            return Ok(update);
        }
        // Not in Python warpsign: also leave off capabilities the team may
        // not have been granted, which the profile check then drops
        let mut skip = unknown_settings.clone();
        for id in &request_only {
            if !skip.contains(id) {
                skip.push(id.clone());
            }
        }
        if skip.is_empty() {
            return Err(PortalError::from_response(&operation, &first));
        }
        let second =
            self.patch_capabilities(bundle, &available, enable, &skip, relationships, &operation)?;
        if second.is_success() {
            update.skipped = unknown_settings;
            update.not_granted = request_only
                .into_iter()
                .filter(|id| !update.skipped.contains(id))
                .collect();
            Ok(update)
        } else {
            Err(PortalError::from_response(&operation, &second))
        }
    }

    fn patch_capabilities(
        &self,
        bundle: &BundleId,
        available: &[&Value],
        enable: &[String],
        skip: &[String],
        relationships: &BTreeMap<String, Vec<String>>,
        operation: &str,
    ) -> Result<Response, PortalError> {
        let mut capabilities = Vec::new();
        for capability in available {
            let Some(id) = capability.get("id").and_then(Value::as_str) else {
                continue;
            };
            let optional = capability
                .pointer("/attributes/optional")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let requested = enable.iter().any(|e| e == id) && !skip.iter().any(|s| s == id);
            let enabled = requested || !optional;
            let settings = if enabled {
                capability_settings(id).unwrap_or_else(|| json!([]))
            } else {
                json!([])
            };
            let mut links = json!({"capability": {"data": {"type": "capabilities", "id": id}}});
            if enabled
                && let Some(kind) = relationship_type(id)
                && let Some(ids) = relationships.get(id).filter(|ids| !ids.is_empty())
            {
                links[kind] = json!({
                    "data": ids.iter().map(|rid| json!({"id": rid, "type": kind})).collect::<Vec<_>>()
                });
            }
            capabilities.push(json!({
                "type": "bundleIdCapabilities",
                "attributes": {"enabled": enabled, "settings": settings},
                "relationships": links,
            }));
        }

        let url = format!("{V1}/bundleIds/{}", bundle.id);
        let request = Request::new(Method::Patch, &url)
            .headers(&[
                (
                    "Referer",
                    &format!(
                        "https://developer.apple.com/account/resources/identifiers/bundleId/edit/{}",
                        bundle.id
                    ),
                ),
                ("Origin", "https://developer.apple.com"),
                ("Sec-Fetch-Dest", "empty"),
                ("Sec-Fetch-Mode", "cors"),
                ("Sec-Fetch-Site", "same-origin"),
                ("Accept", "application/json, text/plain, */*"),
                ("Accept-Language", "en-US,en;q=0.5"),
                ("Content-Type", "application/vnd.api+json"),
                // Python warpsign's header, named like this (not
                // X-Requested-With) only on this request
                ("XMLHttpRequest", "XMLHttpRequest"),
            ])
            .json(&json!({
                "data": {
                    "type": "bundleIds",
                    "id": bundle.id,
                    "attributes": {
                        "identifier": bundle.identifier,
                        "permissions": {"edit": true, "delete": true},
                        "seedId": self.team_id,
                        "name": bundle.identifier,
                        "wildcard": false,
                        "teamId": self.team_id,
                    },
                    "relationships": {"bundleIdCapabilities": {"data": capabilities}},
                }
            }));
        self.send(self.with_csrf(request), operation)
    }

    pub fn app_groups(&self) -> Result<Vec<AppGroup>, PortalError> {
        let operation = "listing app groups";
        let request = Request::post(format!(
            "{LEGACY}/ios/identifiers/listApplicationGroups.action"
        ))
        .headers(&[
            ("Accept", "application/json, text/plain, */*"),
            ("Accept-Language", "en-US,en;q=0.5"),
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("X-Requested-With", "XMLHttpRequest"),
        ])
        .raw_body(format!(
            "onlyCountLists=true&pageSize=1000&pageNumber=1&sort=name%3Dasc&teamId={}",
            self.team_id
        ));
        let body = self.expect_legacy_json(request, operation)?;
        Ok(body
            .get("applicationGroupList")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|group| {
                Some(AppGroup {
                    id: string(group, "applicationGroup")?,
                    identifier: string(group, "identifier")?,
                    name: string(group, "name").unwrap_or_default(),
                })
            })
            .collect())
    }

    /// Register an app group, or find it if this team already has it
    pub fn register_app_group(
        &self,
        identifier: &str,
        name: &str,
    ) -> Result<AppGroup, PortalError> {
        let operation = format!("registering app group {identifier}");
        let body = self.register_legacy(
            &format!("{LEGACY}/ios/identifiers/addApplicationGroup.action"),
            identifier,
            name,
            &operation,
        )?;
        if is_taken(&body) {
            return self
                .app_groups()?
                .into_iter()
                .find(|group| group.identifier == identifier)
                .ok_or_else(|| {
                    PortalError::new(&operation, None, "it's taken, and not by this team")
                });
        }
        check_result_code(&body, &operation)?;
        let group = body.get("applicationGroup");
        group
            .and_then(|group| {
                Some(AppGroup {
                    id: string(group, "applicationGroup")?,
                    identifier: string(group, "identifier")?,
                    name: string(group, "name").unwrap_or_default(),
                })
            })
            .ok_or_else(|| PortalError::new(&operation, None, "no app group in the response"))
    }

    pub fn cloud_containers(&self) -> Result<Vec<CloudContainer>, PortalError> {
        let body = self.list("cloudContainers", "limit=1000", "listing iCloud containers")?;
        resources(&body, "listing iCloud containers", |item| {
            Some(CloudContainer {
                id: string(item, "id")?,
                identifier: string(item.get("attributes")?, "identifier")?,
                name: string(item.get("attributes")?, "name").unwrap_or_default(),
            })
        })
    }

    /// Register an iCloud container, or find it if this team already has it
    pub fn register_cloud_container(
        &self,
        identifier: &str,
        name: &str,
    ) -> Result<CloudContainer, PortalError> {
        let operation = format!("registering iCloud container {identifier}");
        let body = self.register_legacy(
            &format!("{LEGACY}/cloudContainer/addCloudContainer.action"),
            identifier,
            name,
            &operation,
        )?;
        if is_taken(&body) {
            return self
                .cloud_containers()?
                .into_iter()
                .find(|container| container.identifier == identifier)
                .ok_or_else(|| {
                    PortalError::new(&operation, None, "it's taken, and not by this team")
                });
        }
        check_result_code(&body, &operation)?;
        body.get("cloudContainer")
            .and_then(|container| {
                Some(CloudContainer {
                    id: string(container, "cloudContainer")?,
                    identifier: string(container, "identifier")?,
                    name: string(container, "name").unwrap_or_default(),
                })
            })
            .ok_or_else(|| {
                PortalError::new(&operation, None, "no iCloud container in the response")
            })
    }

    /// Enabled iPhones and iPads
    pub fn devices(&self) -> Result<Vec<Device>, PortalError> {
        let body = self.list(
            "devices",
            "limit=1000&offset=0&filter[status]=ENABLED",
            "listing devices",
        )?;
        let devices = resources(&body, "listing devices", device_from)?;
        Ok(devices
            .into_iter()
            .filter(|device| matches!(device.device_class.as_str(), "IPHONE" | "IPAD"))
            .collect())
    }

    /// Register an iPhone or iPad by UDID. Not in Python warpsign: the
    /// request follows the App Store Connect API that the portal fronts.
    pub fn register_device(&self, name: &str, udid: &str) -> Result<Device, PortalError> {
        let operation = format!("registering device {udid}");
        let request = self
            .write_json(Request::post(format!("{V1}/devices")))
            .json(&json!({
                "data": {
                    "type": "devices",
                    "attributes": {
                        "name": name,
                        "udid": udid,
                        "platform": "IOS",
                        "teamId": self.team_id,
                    },
                }
            }));
        let body = self.expect_json(request, &operation)?;
        body.get("data")
            .and_then(device_from)
            .ok_or_else(|| PortalError::new(&operation, None, "no device in the response"))
    }

    pub fn profiles(&self) -> Result<Vec<PortalProfile>, PortalError> {
        let body = self.list(
            "profiles",
            "limit=1000&fields[profiles]=name,platform,platformName,profileTypeLabel,expirationDate,profileState&sort=name",
            "listing provisioning profiles",
        )?;
        resources(&body, "listing provisioning profiles", |item| {
            let attributes = item.get("attributes")?;
            Some(PortalProfile {
                id: string(item, "id")?,
                name: string(attributes, "name")?,
                state: string(attributes, "profileState").unwrap_or_default(),
                platform: string(attributes, "platform").unwrap_or_default(),
                type_label: string(attributes, "profileTypeLabel").unwrap_or_default(),
                expiration_date: string(attributes, "expirationDate"),
            })
        })
    }

    /// Create a provisioning profile, or regenerate `existing_id` in place,
    /// and download it. Returns its portal ID and contents.
    #[allow(clippy::too_many_arguments)]
    pub fn create_or_regenerate_profile(
        &self,
        existing_id: Option<&str>,
        bundle: &BundleId,
        name: &str,
        certificate_ids: &[String],
        device_ids: &[String],
        profile_type: ProfileType,
    ) -> Result<(String, Vec<u8>), PortalError> {
        let operation = format!("creating provisioning profile \"{name}\"");
        let distribution_type = match profile_type {
            ProfileType::Development => "limited",
            ProfileType::AdHoc => "adhoc",
        };
        let request = Request::post(format!(
            "{LEGACY}/ios/profile/regenProvisioningProfile.action"
        ))
        .headers(&[
            ("Accept", "application/json, text/javascript, */*; q=0.01"),
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("X-Requested-With", "XMLHttpRequest"),
        ])
        .form(&[
            ("appIdId", &bundle.id),
            ("provisioningProfileId", existing_id.unwrap_or("")),
            ("distributionType", distribution_type),
            ("provisioningProfileName", name),
            ("certificateIds", &certificate_ids.join(",")),
            ("deviceIds", &device_ids.join(",")),
            ("teamId", &self.team_id),
            ("subPlatform", ""),
            ("isExcludeAudiences", ""),
            ("returnFullObjects", "false"),
        ]);
        let body = self.expect_legacy_json(self.with_csrf(request), &operation)?;
        let profile_id = body
            .pointer("/provisioningProfile/provisioningProfileId")
            .and_then(Value::as_str)
            .ok_or_else(|| PortalError::new(&operation, None, "no profile ID in the response"))?;
        Ok((profile_id.to_string(), self.download_profile(profile_id)?))
    }

    pub fn download_profile(&self, profile_id: &str) -> Result<Vec<u8>, PortalError> {
        let operation = "downloading a provisioning profile";
        let request = Request::get(format!("{LEGACY}/ios/profile/downloadProfileContent"))
            .query(&[
                ("teamId", &self.team_id),
                ("provisioningProfileId", profile_id),
            ])
            .headers(&[("Accept", "*/*"), ("X-Requested-With", "XMLHttpRequest")]);
        let response = self.send(request, operation)?;
        if !response.is_success() {
            return Err(PortalError::from_response(operation, &response));
        }
        Ok(response.body)
    }

    // Requests

    fn send(&self, request: Request, operation: &str) -> Result<Response, PortalError> {
        self.http
            .send(request)
            .map_err(|e| PortalError::transport(operation, e))
    }

    fn expect_json(&self, request: Request, operation: &str) -> Result<Value, PortalError> {
        let response = self.send(request, operation)?;
        json_from(&response, operation)
    }

    /// For the older `.action` endpoints, which report errors in the body
    fn expect_legacy_json(&self, request: Request, operation: &str) -> Result<Value, PortalError> {
        let body = self.expect_json(request, operation)?;
        check_result_code(&body, operation)?;
        Ok(body)
    }

    /// A `v1` read: a POST that asks to be treated as a GET, with the query
    /// in the body
    fn read(&self, url: &str, query: &str) -> Request {
        Request::post(url)
            .headers(&[
                ("Accept", "application/json, text/plain, */*"),
                ("Accept-Language", "en-US,en;q=0.5"),
                ("Content-Type", "application/vnd.api+json"),
                ("X-Requested-With", "XMLHttpRequest"),
                ("X-HTTP-Method-Override", "GET"),
            ])
            .json(&json!({"urlEncodedQueryParams": query, "teamId": self.team_id}))
    }

    fn list(&self, resource: &str, query: &str, operation: &str) -> Result<Value, PortalError> {
        self.expect_json(self.read(&format!("{V1}/{resource}"), query), operation)
    }

    /// Headers for `v1` changes
    fn write_json(&self, request: Request) -> Request {
        self.with_csrf(request.headers(&[
            ("Origin", "https://developer.apple.com"),
            ("Sec-Fetch-Dest", "empty"),
            ("Sec-Fetch-Mode", "cors"),
            ("Sec-Fetch-Site", "same-origin"),
            ("Accept", "application/json, text/plain, */*"),
            ("Accept-Language", "en-US,en;q=0.5"),
            ("Content-Type", "application/vnd.api+json"),
            ("X-Requested-With", "XMLHttpRequest"),
        ]))
    }

    /// The anti-CSRF tokens the portal wants on changes
    fn with_csrf(&self, mut request: Request) -> Request {
        let state = self.http.state();
        if let Some(csrf) = state.csrf {
            request = request.header("csrf", csrf);
        }
        if let Some(csrf_ts) = state.csrf_ts {
            request = request.header("csrf_ts", csrf_ts);
        }
        request
    }

    /// Register an app group or iCloud container with an `.action` form
    fn register_legacy(
        &self,
        url: &str,
        identifier: &str,
        name: &str,
        operation: &str,
    ) -> Result<Value, PortalError> {
        let request = Request::post(url)
            .headers(&[
                ("Origin", "https://developer.apple.com"),
                ("Sec-Fetch-Dest", "empty"),
                ("Sec-Fetch-Mode", "cors"),
                ("Sec-Fetch-Site", "same-origin"),
                ("Accept", "application/json, text/plain, */*"),
                ("Accept-Language", "en-US,en;q=0.5"),
                ("Content-Type", "application/x-www-form-urlencoded"),
                ("X-Requested-With", "XMLHttpRequest"),
            ])
            .form(&[
                ("name", name),
                ("identifier", identifier),
                ("teamId", &self.team_id),
            ]);
        self.expect_json(self.with_csrf(request), operation)
    }
}

/// The teams this Apple ID belongs to
pub fn fetch_teams(http: &HttpClient) -> Result<Vec<Team>, PortalError> {
    let operation = "listing teams";
    let request = Request::post(format!("{SERVICES}/QH65B2/account/getTeams"))
        .headers(&[
            ("Accept", "application/json, text/javascript"),
            ("Content-Type", "application/json"),
            ("X-Requested-With", "XMLHttpRequest"),
        ])
        .json(&json!({"includeInMigrationTeams": 1}));
    let response = http
        .send(request)
        .map_err(|e| PortalError::transport(operation, e))?;
    let body = json_from(&response, operation)?;
    check_result_code(&body, operation)?;
    Ok(body
        .get("teams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|team| {
            Some(Team {
                team_id: string(team, "teamId")?,
                name: string(team, "name").unwrap_or_default(),
                status: string(team, "status").unwrap_or_default(),
                kind: string(team, "entityType").unwrap_or_default(),
                roles: team
                    .get("userRoles")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect(),
            })
        })
        .collect())
}

fn json_from(response: &Response, operation: &str) -> Result<Value, PortalError> {
    if !response.is_success() {
        return Err(PortalError::from_response(operation, response));
    }
    response.json().ok_or_else(|| {
        PortalError::new(
            operation,
            Some(response.status),
            "the response isn't JSON (signed out?)",
        )
    })
}

fn check_result_code(body: &Value, operation: &str) -> Result<(), PortalError> {
    match body.get("resultCode").and_then(Value::as_i64) {
        Some(0) | None => Ok(()),
        Some(code) => {
            let message = super::apple_error_message(body.to_string().as_bytes())
                .unwrap_or_else(|| format!("result code {code}"));
            Err(PortalError::new(operation, None, message))
        }
    }
}

/// The `.action` endpoints' way of saying an identifier is already used
fn is_taken(body: &Value) -> bool {
    body.get("userString")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("is not available. Please enter a different string."))
}

fn resources<T>(
    body: &Value,
    operation: &str,
    parse: impl Fn(&Value) -> Option<T>,
) -> Result<Vec<T>, PortalError> {
    let items = body
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| PortalError::new(operation, None, "no data in the response"))?;
    Ok(items.iter().filter_map(parse).collect())
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(String::from)
}

fn certificate_from(item: &Value) -> Option<PortalCertificate> {
    let attributes = item.get("attributes")?;
    Some(PortalCertificate {
        id: string(item, "id")?,
        serial_number: string(attributes, "serialNumber")?,
        certificate_type: string(attributes, "certificateType").unwrap_or_default(),
        name: string(attributes, "name").unwrap_or_default(),
        display_name: string(attributes, "displayName"),
        owner_id: string(attributes, "ownerId"),
        expiration_date: string(attributes, "expirationDate"),
    })
}

fn content_from(item: &Value) -> Option<Vec<u8>> {
    let content = item.pointer("/attributes/certificateContent")?.as_str()?;
    BASE64.decode(content.trim()).ok()
}

fn device_from(item: &Value) -> Option<Device> {
    let attributes = item.get("attributes")?;
    Some(Device {
        id: string(item, "id")?,
        name: string(attributes, "name").unwrap_or_default(),
        udid: string(attributes, "udid")?,
        status: string(attributes, "status").unwrap_or_default(),
        device_class: string(attributes, "deviceClass").unwrap_or_default(),
        platform: string(attributes, "platform").unwrap_or_default(),
        model: string(attributes, "model"),
    })
}

fn bundle_id_from(item: &Value) -> Option<BundleId> {
    let attributes = item.get("attributes")?;
    Some(BundleId {
        id: string(item, "id")?,
        identifier: string(attributes, "identifier")?,
        name: string(attributes, "name").unwrap_or_default(),
    })
}

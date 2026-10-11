//! Core logic for warpsign-neo, shared by the CLI and the iOS app.

pub mod bundle_mapping;
pub mod entitlements;
pub mod info_plist;
pub mod inspect;
pub mod ipa;
pub mod pipeline;
pub mod profile;
pub mod progress;
mod pyrandom;
pub mod signature;
pub mod signing;

#[doc(hidden)]
pub use pyrandom::PyRandom;

/// Kind of provisioning profile being signed with
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProfileType {
    /// "Apple Development" certificate
    Development,
    /// "Apple Distribution" certificate
    AdHoc,
}

impl std::str::FromStr for ProfileType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "development" => Ok(Self::Development),
            "adhoc" => Ok(Self::AdHoc),
            other => Err(format!("unknown profile type: {other}")),
        }
    }
}

//! Reads existing code signatures from Mach-O binaries.
//!
//! Replaces Python warpsign's use of `ldid -e`. Like ldid, only the first
//! architecture of a universal binary is read.

use std::path::Path;

use apple_codesign::MachFile;
use plist::{Dictionary, Value};

#[derive(Debug, thiserror::Error)]
pub enum SignatureError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("parsing the signature of {path}: {message}")]
    Parse { path: String, message: String },
}

/// The parts of an embedded code signature warpsign cares about
#[derive(Debug, Clone, PartialEq)]
pub struct SignatureInfo {
    /// Code signing identifier (normally the bundle ID)
    pub identifier: String,
    /// Team ID recorded in the code directory, if any
    pub team_id: Option<String>,
    /// Entitlements from the XML entitlements blob (empty if there is none)
    pub entitlements: Dictionary,
}

/// Read the signature of a Mach-O binary. Returns `None` if it isn't signed.
pub fn read_signature(path: &Path) -> Result<Option<SignatureInfo>, SignatureError> {
    let display = path.display().to_string();
    let parse_error = |message: String| SignatureError::Parse {
        path: display.clone(),
        message,
    };

    let data = std::fs::read(path).map_err(|source| SignatureError::Io {
        path: display.clone(),
        source,
    })?;
    let mach = MachFile::parse(&data).map_err(|e| parse_error(e.to_string()))?;
    let Some(macho) = mach.iter_macho().next() else {
        return Ok(None);
    };
    let Some(signature) = macho
        .code_signature()
        .map_err(|e| parse_error(e.to_string()))?
    else {
        return Ok(None);
    };

    let code_directory = signature
        .code_directory()
        .map_err(|e| parse_error(e.to_string()))?
        .ok_or_else(|| parse_error("signature has no code directory".into()))?;

    let entitlements = match signature
        .entitlements()
        .map_err(|e| parse_error(e.to_string()))?
    {
        Some(blob) => match Value::from_reader_xml(blob.as_str().as_bytes())
            .map_err(|e| parse_error(format!("entitlements are not a valid plist: {e}")))?
        {
            Value::Dictionary(dict) => dict,
            _ => return Err(parse_error("entitlements are not a dictionary".into())),
        },
        None => Dictionary::new(),
    };

    Ok(Some(SignatureInfo {
        identifier: code_directory.ident.to_string(),
        team_id: code_directory.team_name.as_ref().map(|t| t.to_string()),
        entitlements,
    }))
}

/// Entitlements a binary is signed with; empty if it is unsigned or has none
pub fn read_entitlements(path: &Path) -> Result<Dictionary, SignatureError> {
    Ok(read_signature(path)?
        .map(|info| info.entitlements)
        .unwrap_or_default())
}

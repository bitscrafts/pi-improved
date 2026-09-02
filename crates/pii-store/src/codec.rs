//! `serde_json` codec helpers (pass-1 §3.4 F3.2).

use pii_abi::PiiError;
use serde::{Serialize, de::DeserializeOwned};

/// Serialise a value to JSON bytes, mapping errors to [`PiiError::Internal`].
///
/// # Errors
/// Returns [`PiiError::Internal`] if serialisation fails.
pub fn to_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, PiiError> {
    serde_json::to_vec(value).map_err(|e| PiiError::Internal(format!("store: encode: {e}")))
}

/// Deserialise a value from JSON bytes, mapping errors to [`PiiError::Internal`].
///
/// # Errors
/// Returns [`PiiError::Internal`] if deserialisation fails.
pub fn from_bytes<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, PiiError> {
    serde_json::from_slice(bytes).map_err(|e| PiiError::Internal(format!("store: decode: {e}")))
}

/// blake3 hex hash of a byte slice.
#[must_use]
pub fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

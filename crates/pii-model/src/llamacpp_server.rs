//! The `llamacpp-server:<model>` kind (spec 129): a llama.cpp *server* spoken to
//! over HTTP through its OpenAI-compatible API.
//!
//! This is not [`crate::llamacpp`], which loads a GGUF file in-process and has
//! no host. The server kind reuses [`OpenAiProvider`]; the base URL comes from
//! the config's `[llama] host` (default loopback) rather than an environment
//! variable, and no API key is sent (local servers take none).

use std::sync::Arc;

use pii_abi::PiiError;
use pii_core::HttpTransport;
use pii_observe::Observer;

use crate::{ModelProvider, OpenAiProvider};

/// Default `[llama] host` when none is configured: loopback.
pub const DEFAULT_LLAMA_HOST: &str = "http://127.0.0.1:8080/v1";

/// Builds a `llamacpp-server:<model>` provider against `host` (`None` =
/// [`DEFAULT_LLAMA_HOST`]).
///
/// # Errors
/// [`PiiError::Invalid`] for an empty model name or a foreign spec kind.
pub fn provider(
    spec: &str,
    transport: Arc<dyn HttpTransport>,
    host: Option<&str>,
    observer: Option<Arc<dyn Observer>>,
) -> Result<Arc<dyn ModelProvider>, PiiError> {
    let model = spec
        .strip_prefix("llamacpp-server:")
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            PiiError::Invalid("llamacpp-server: spec must be llamacpp-server:<model>".into())
        })?;
    let base = host.unwrap_or(DEFAULT_LLAMA_HOST);
    let provider = OpenAiProvider::new(transport, base, model.to_owned(), None);
    Ok(match observer {
        Some(o) => Arc::new(provider.with_observer(o)),
        None => Arc::new(provider),
    })
}

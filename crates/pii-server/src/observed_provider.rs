//! Resolving and wrapping the engine's model provider (specs 108, 129).

use std::sync::Arc;

use pii_abi::PiiError;
use pii_model::ModelProvider;

/// Resolves the model provider for `model`, wrapped to report to `log` (spec 108).
///
/// Returns the provider and, for a networked spec, the host its transport was
/// scoped to. `llama_host` is `[llama] host` for `llamacpp-server:` specs.
///
/// # Errors
/// [`PiiError::Invalid`] for a retired `ollama:` spec (also counted), an
/// unknown spec, or a provider that cannot be built.
pub fn observed_provider(
    model: &str,
    log: &Arc<pii_observe::SwitchObserver>,
    llama_host: Option<&str>,
) -> Result<(Arc<dyn ModelProvider>, Option<String>), PiiError> {
    // The retired `ollama:` kind is refused loudly (spec 129): counted, then
    // the named error from `provider_from_spec`, never a fallback.
    if model.starts_with("ollama:") {
        pii_observe::Observer::event(
            &**log,
            &pii_observe::Event::new(
                "model",
                pii_observe::Level::Error,
                pii_observe::names::MODEL_OLLAMA_REFUSED,
            )
            .field("model", model),
        );
        return Err(pii_model::provider_from_spec(model)
            .err()
            .unwrap_or_else(|| PiiError::Invalid(String::from("ollama: is retired"))));
    }
    // `llamacpp-server:<model>` (and the hosted kinds) construct a live
    // provider. The transport is a dedicated `pii-net::NetService`, scoped by
    // a single grant for exactly the configured host (`[llama] host`,
    // loopback by default) — this
    // is the ONE place besides the daemon's own `[net]` config that
    // opens a `NetService`, and it exists specifically so the model
    // provider does not bypass the egress chokepoint `cargo xtask
    // net-check` enforces. `scripted:` specs (the common case in
    // tests) never touch this — `resolve_with_transport` only builds
    // the transport lazily, inside the networked-kind branch
    // (`llamacpp-server:`, `openai:`, `anthropic:`, `gemini:` — spec 111).
    let mut local_net_host: Option<String> = None;
    let provider = if let Some(host_only) = pii_model::egress_host_with(model, llama_host) {
        let mut grants = pii_net::grant::StaticGrants::default();
        grants.insert(
            "*",
            &format!("http:{host_only}"),
            pii_net::grant::NetGrant::open_local(),
        );
        let net = pii_net::NetService::new(pii_net::NetConfig {
            resolver: Arc::new(pii_net::SystemResolver),
            grants: Arc::new(grants),
            clock: Arc::new(pii_net::SystemClock),
        });
        net.set_observer(log.clone());
        local_net_host = Some(host_only);
        // `llamacpp-server:` reports through `ObservedProvider` alone; the hosted kinds
        // also own named `model.provider.*` events and need the observer.
        if model.starts_with("llamacpp-server:") {
            pii_model::resolve_with_transport(model, Arc::new(net), llama_host)?
        } else {
            pii_model::resolve_with_observer(model, Arc::new(net), log.clone())?
        }
    } else {
        pii_model::provider_from_spec(model)?
    };
    let provider: Arc<dyn ModelProvider> =
        Arc::new(pii_model::ObservedProvider::new(provider, log.clone()));
    Ok((provider, local_net_host))
}

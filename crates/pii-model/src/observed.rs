//! [`ObservedProvider`]: wraps any [`ModelProvider`] so every request reports
//! the provider in force, its latency, and any provider error through the
//! `pii-observe` contract, and so the model actually resolved is reportable
//! as [`Health`].

use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use pii_abi::PiiError;
use pii_core::Ctx;
use pii_observe::{
    Event, Health, Level, Measure, Observable, Observed, Observer, SubsystemHealth, names,
};

use crate::{ChatRequest, ChatResponse, ModelProvider};

/// A [`ModelProvider`] that reports to an [`Observer`] injected at construction.
pub struct ObservedProvider {
    inner: Arc<dyn ModelProvider>,
    observer: Arc<dyn Observer>,
    /// The model named by the most recent request, once there has been one.
    last_model: Mutex<Option<String>>,
}

impl ObservedProvider {
    /// Wraps `inner`; every request is reported to `observer`.
    #[must_use]
    pub fn new(inner: Arc<dyn ModelProvider>, observer: Arc<dyn Observer>) -> Self {
        Self {
            inner,
            observer,
            last_model: Mutex::new(None),
        }
    }

    fn observe(
        &self,
        req: &ChatRequest,
        started: Instant,
        result: &Result<ChatResponse, PiiError>,
    ) {
        *self.last_model.lock() = Some(req.model.clone());
        self.emit(
            &Event::new("model", Level::Info, names::MODEL_PROVIDER_IN_FORCE)
                .field("provider", self.inner.name())
                .field("model", req.model.as_str()),
        );
        let us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        self.record(&Measure::Histogram {
            count: 1,
            sum_us: us,
            max_us: us,
        });
        self.emit(
            &Event::new("model", Level::Debug, names::MODEL_REQUEST_LATENCY)
                .field("provider", self.inner.name())
                .field("us", us),
        );
        if let Err(e) = result {
            self.emit(
                &Event::new("model", Level::Error, names::MODEL_PROVIDER_ERROR)
                    .field("provider", self.inner.name())
                    .field("error", e.to_string()),
            );
        }
    }
}

impl Observable for ObservedProvider {
    fn observer(&self) -> Option<&dyn Observer> {
        Some(self.observer.as_ref())
    }
}

impl ModelProvider for ObservedProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn chat(&self, ctx: &Ctx, req: &ChatRequest) -> Result<ChatResponse, PiiError> {
        let started = Instant::now();
        let result = self.inner.chat(ctx, req);
        self.observe(req, started, &result);
        result
    }

    fn chat_streaming(
        &self,
        ctx: &Ctx,
        req: &ChatRequest,
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<ChatResponse, PiiError> {
        let started = Instant::now();
        let result = self.inner.chat_streaming(ctx, req, on_delta);
        self.observe(req, started, &result);
        result
    }
}

impl Health for ObservedProvider {
    fn health(&self) -> SubsystemHealth {
        let mut h = SubsystemHealth::ok("model");
        h.identity
            .insert("provider".into(), self.inner.name().to_owned());
        if let Some(m) = self.last_model.lock().as_ref() {
            h.identity.insert("model".into(), m.clone());
        }
        h
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use pii_core::{Budget, CapSet};
    use pii_observe::Recorder;

    use super::*;
    use crate::ScriptedProvider;

    fn req() -> ChatRequest {
        ChatRequest {
            model: "m1".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: 1,
            temperature: 0.0,
            num_ctx: 0,
        }
    }

    #[test]
    fn provider_in_force_latency_and_error_reach_the_observer_by_name() {
        let rec = Arc::new(Recorder::new());
        // `empty()` has no turns: the first chat fails (a provider error).
        let p = ObservedProvider::new(Arc::new(ScriptedProvider::empty()), rec.clone());
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        assert!(p.chat(&ctx, &req()).is_err());
        let in_force = rec.events_named(names::MODEL_PROVIDER_IN_FORCE);
        assert_eq!(in_force.len(), 1);
        assert_eq!(in_force[0].fields.get("provider"), Some(&"scripted".into()));
        assert_eq!(in_force[0].fields.get("model"), Some(&"m1".into()));
        assert_eq!(rec.events_named(names::MODEL_REQUEST_LATENCY).len(), 1);
        assert_eq!(rec.events_named(names::MODEL_PROVIDER_ERROR).len(), 1);
        assert_eq!(rec.measures().len(), 1);
        let h = p.health();
        assert_eq!(
            h.identity.get("provider").map(String::as_str),
            Some("scripted")
        );
        assert_eq!(h.identity.get("model").map(String::as_str), Some("m1"));
    }
}

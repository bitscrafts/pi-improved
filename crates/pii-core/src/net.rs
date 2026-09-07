//! The HTTP transport seam (F1.4): lives in `pii-core`, not `pii-net`, so
//! `pii-model` and the channel router can consume HTTP **without** a
//! dependency on the socket crate. The only implementation is
//! `pii_net::NetService`; tests use hand-rolled fakes — no network.

use pii_abi::{HttpRequest, HttpResponse, PiiError};

use crate::Ctx;

/// A synchronous HTTP client. Sync on purpose: `GraphNode::run` is sync and
/// executes on a blocking thread, matching [`crate::Ctx`] enforcement.
pub trait HttpTransport: Send + Sync {
    /// Performs one HTTP request. The single deadline (`req.timeout_ms`)
    /// spans authorize, resolve, dial, write and read.
    ///
    /// # Errors
    /// [`PiiError::Denied`] when egress policy refuses the target;
    /// [`PiiError::Unavailable`] when no address is reachable or the peer
    /// fails transiently; [`PiiError::Invalid`] for a malformed URL or
    /// response.
    fn request(&self, ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError>;

    /// Performs one HTTP request, forwarding every body chunk as it arrives.
    /// Streaming is additive and opt-in (spec 4.1c): the default delegates to
    /// [`HttpTransport::request`] and delivers the full body as one chunk, so
    /// existing transports keep working unchanged.
    ///
    /// # Errors
    /// Same contract as [`HttpTransport::request`].
    fn request_streaming(
        &self,
        ctx: &Ctx,
        req: &HttpRequest,
        on_chunk: &mut dyn FnMut(&[u8]),
    ) -> Result<HttpResponse, PiiError> {
        let resp = self.request(ctx, req)?;
        on_chunk(&resp.body);
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::HttpTransport;
    use crate::{Budget, CapKind, CapSet, Ctx};
    use pii_abi::{HttpRequest, HttpResponse, PiiError};
    use std::sync::Arc;

    struct Fake;

    impl HttpTransport for Fake {
        fn request(&self, _ctx: &Ctx, _req: &HttpRequest) -> Result<HttpResponse, PiiError> {
            Ok(HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: b"ok".to_vec(),
            })
        }
    }

    #[test]
    fn transport_serves_without_network() {
        let ctx = Ctx::root(CapSet::empty(), Budget::unlimited());
        let t: Arc<dyn HttpTransport> = Arc::new(Fake);
        let r = t
            .request(
                &ctx,
                &HttpRequest {
                    method: pii_abi::HttpMethod::Get,
                    url: "http://example/x".into(),
                    headers: Vec::new(),
                    body: Vec::new(),
                    timeout_ms: 1_000,
                },
            )
            .unwrap();
        assert_eq!(r.body, b"ok");
        assert_eq!(CapKind::NET, CapKind::NET);
    }
}

//! `impl HttpTransport` for `NetService`: buffered + streaming request paths.

use pii_abi::{HttpRequest, HttpResponse, PiiError};
use pii_core::{Ctx, HttpTransport};

use super::NetService;
use super::dechunk::{dechunk_step, read_to_deadline};
use crate::authorize::Pinned;
use crate::http_codec::{parse_response, parse_url};

impl HttpTransport for NetService {
    fn request(&self, ctx: &Ctx, req: &HttpRequest) -> Result<HttpResponse, PiiError> {
        let (scheme, target, path) = parse_url(&req.url)?;
        let host_header = format!("{}:{}", target.host, target.port);
        let key = format!("http:{}", target.host);
        // connect() resolves exactly once and authorizes against the SAME
        // pinned set — never re-resolve here (§4.3.2).
        let mut conn = self.connect(ctx, &target, &key, req.timeout_ms, scheme)?;
        let pinned = Pinned {
            addrs: Vec::new(),
            deadline_ms: self.0.clock.now_ms().saturating_add(req.timeout_ms),
        };
        conn.arm_deadline(&pinned, self.0.clock.as_ref())?;
        let wire = crate::http_codec::render_request(req, &host_header, &path);
        conn.write_all(&wire)?;
        let raw = read_to_deadline(&mut conn)?;
        parse_response(&raw)
    }

    /// Streaming client path (spec 4.1c): same single connect + write as
    /// [`NetService::request`], but each socket read is forwarded through
    /// `on_chunk` as it arrives. The returned [`HttpResponse`] is assembled
    /// from the same bytes, so non-streaming consumers see no difference.
    /// Reuses `Conn::read` — no new socket path, byte accounting intact.
    #[allow(clippy::too_many_lines)] // reason: dechunk framing + deadline
    // accounting is one cohesive wire loop; splitting it would hide the
    // byte-budget invariants this function exists to enforce
    fn request_streaming(
        &self,
        ctx: &Ctx,
        req: &HttpRequest,
        on_chunk: &mut dyn FnMut(&[u8]),
    ) -> Result<HttpResponse, PiiError> {
        let (scheme, target, path) = parse_url(&req.url)?;
        let host_header = format!("{}:{}", target.host, target.port);
        let key = format!("http:{}", target.host);
        let mut conn = self.connect(ctx, &target, &key, req.timeout_ms, scheme)?;
        let pinned = Pinned {
            addrs: Vec::new(),
            deadline_ms: self.0.clock.now_ms().saturating_add(req.timeout_ms),
        };
        conn.arm_deadline(&pinned, self.0.clock.as_ref())?;
        let wire = crate::http_codec::render_request(req, &host_header, &path);
        conn.write_all(&wire)?;
        let mut head: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        let head_end = loop {
            match conn.read(&mut buf) {
                Ok(0) => return Err(PiiError::Invalid("http: truncated response".into())),
                Ok(n) => {
                    head.extend_from_slice(buf.get(..n).unwrap_or(&buf));
                    if head.len() > crate::limits::MAX_HTTP_BODY {
                        return Err(PiiError::Invalid("http: body too large".into()));
                    }
                    if let Some(i) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                }
                Err(e) => return Err(e),
            }
        };
        let (status, headers, _body_start) = crate::http_codec::parse_response_head(&head)?;
        // R3 (spec 22 A3): when the response is chunked, `on_chunk` must
        // receive DECHUNKED body bytes — not the hex size lines. Reuses
        // `read_chunked`'s incremental state machine; the returned
        // HttpResponse.body is identical to the buffered `request` path.
        let chunked = headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
        });
        // Stream everything after the head (plus what arrived early).
        let mut pending: Vec<u8> = head.get(head_end..).unwrap_or(&[]).to_vec();
        let mut total: Vec<u8> = Vec::new();
        let mut consumed: usize = 0; // bytes of `pending` consumed by chunk framing
        if chunked {
            // Incremental dechunk: parse as many complete chunks as the
            // bytes seen so far allow, forwarding decoded bytes only.
            dechunk_step(&pending, &mut consumed, &mut |d| {
                total.extend_from_slice(d);
                on_chunk(d);
            })?;
            pending.drain(..consumed);
            consumed = 0;
        } else {
            total.extend_from_slice(&pending);
            on_chunk(&pending);
            pending.clear();
        }
        loop {
            match conn.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let sl = buf.get(..n).unwrap_or(&buf);
                    pending.extend_from_slice(sl);
                    if pending.len() > crate::limits::MAX_HTTP_BODY {
                        return Err(PiiError::Invalid("http: body too large".into()));
                    }
                    if chunked {
                        dechunk_step(&pending, &mut consumed, &mut |d| {
                            total.extend_from_slice(d);
                            on_chunk(d);
                        })?;
                        pending.drain(..consumed);
                        consumed = 0;
                    } else {
                        total.extend_from_slice(sl);
                        on_chunk(sl);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        if chunked {
            // The final 0-size chunk may still be unprocessed.
            dechunk_step(&pending, &mut consumed, &mut |d| {
                total.extend_from_slice(d);
                on_chunk(d);
            })?;
        }
        Ok(HttpResponse {
            status,
            headers,
            body: total,
        })
    }
}

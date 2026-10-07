//! Incremental chunked-encoding decoding helpers and the deadline-bounded
//! read loop for the HTTP client side.

use pii_abi::PiiError;

use super::conn::Conn;

/// Incremental chunked-encoding decoder: consumes as many COMPLETE chunks
/// as `raw[..*pos]` contains, forwarding decoded body bytes through `out`.
/// State is fully derived from `raw` + `*pos`, so it can be called again
/// after more bytes arrive (resumable across socket reads).
///
/// Split-safety (spec 70): a step may only consume bytes whose framing is
/// fully present — a size line only once its CRLF is in the buffer, a chunk
/// only once data AND its trailing CRLF are (a lone trailing `\r` is never
/// consumed), and the terminating chunk only once `0\r\n\r\n` (plus any
/// full trailer lines) has arrived. A partial frame always means
/// `Ok(())` (wait for more bytes), never a parse error.
pub(super) fn dechunk_step(
    raw: &[u8],
    pos: &mut usize,
    out: &mut dyn FnMut(&[u8]),
) -> Result<(), PiiError> {
    loop {
        let tail = raw.get(*pos..).unwrap_or(&[]);
        if tail.is_empty() {
            return Ok(());
        }
        // R1.1: a size line is parsed only once its CRLF is fully present.
        let Some(rel) = find_crlf(tail) else {
            return Ok(()); // partial line: wait for more bytes
        };
        let size_line = std::str::from_utf8(raw.get(*pos..*pos + rel).unwrap_or(&[]))
            .map_err(|_| PiiError::Invalid("http: bad chunk size".into()))?;
        let size_str = size_line.split(';').next().unwrap_or("0");
        let size = usize::try_from(
            u64::from_str_radix(size_str.trim(), 16)
                .map_err(|_| PiiError::Invalid("http: bad chunk size".into()))?,
        )
        .map_err(|_| PiiError::Invalid("http: body too large".into()))?;
        let data_start = *pos + rel + 2;
        if size == 0 {
            // R1.3: the terminating chunk advances `pos` only once its
            // final CRLF (after any full trailer lines) is in the buffer;
            // a partial trailer line waits.
            let mut p = data_start;
            loop {
                let t = raw.get(p..).unwrap_or(&[]);
                let Some(r) = find_crlf(t) else {
                    return Ok(()); // partial trailer/final CRLF: wait
                };
                p += r + 2;
                if r == 0 {
                    *pos = p; // lands after the final CRLF
                    return Ok(());
                }
            }
        }
        // Hard size gate shared with the chunked reader, unchanged (R3).
        let _gate = total_len(*pos, size)?;
        // R1.2: data + its trailing CRLF must both be present before the
        // chunk may be consumed — `data_start + size + 2 <= raw.len()`.
        // A lone trailing `\r` is therefore never consumed.
        if data_start + size + 2 > raw.len() {
            return Ok(()); // incomplete chunk: wait for more bytes
        }
        let chunk = raw
            .get(data_start..data_start + size)
            .ok_or_else(|| PiiError::Invalid("http: truncated chunk".into()))?;
        out(chunk);
        *pos = data_start + size + 2;
    }
}

/// Hard size gate shared with the chunked reader (`MAX_BODY` + framing).
fn total_len(pos: usize, size: usize) -> Result<usize, PiiError> {
    pos.checked_add(size)
        .and_then(|end| end.checked_add(2))
        .filter(|end| *end <= crate::limits::MAX_BODY + 4)
        .ok_or_else(|| PiiError::Invalid("http: body too large".into()))
}

/// Finds the offset of the first CRLF in `s`.
///
/// R1.4: `windows(2)` only yields windows with BOTH bytes present, so a
/// `\r` alone in the final byte pair (its `\n` not yet arrived) is never
/// mistaken for a complete CRLF.
fn find_crlf(s: &[u8]) -> Option<usize> {
    s.windows(2).position(|w| w == b"\r\n")
}

/// Reads until EOF or the armed timeout, up to 8 MiB (HTTP client side).
pub(super) fn read_to_deadline(conn: &mut Conn) -> Result<Vec<u8>, PiiError> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match conn.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                out.extend_from_slice(buf.get(..n).unwrap_or(&buf));
                if out.len() > crate::limits::MAX_HTTP_BODY {
                    return Err(PiiError::Invalid("http: body too large".into()));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod review_demo {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]
    use pii_abi::PiiError;

    use super::dechunk_step;

    /// Mirrors http.rs: feed bytes incrementally, drain `consumed` after each step.
    fn feed_split(body: &[u8], k: usize) -> Result<Vec<u8>, String> {
        let mut pending: Vec<u8> = Vec::new();
        let mut out = Vec::new();
        for part in [&body[..k], &body[k..]] {
            pending.extend_from_slice(part);
            let mut consumed = 0usize;
            dechunk_step(&pending, &mut consumed, &mut |d| out.extend_from_slice(d))
                .map_err(|e| format!("split at {k}: {e}"))?;
            if consumed > pending.len() {
                return Err(format!(
                    "split at {k}: consumed {consumed} > pending {} (drain would panic)",
                    pending.len()
                ));
            }
            pending.drain(..consumed);
        }
        Ok(out)
    }

    #[test]
    fn every_split_point_decodes() {
        let body = b"5\r\nhello\r\n0\r\n\r\n";
        let mut failures = Vec::new();
        for k in 0..=body.len() {
            match feed_split(body, k) {
                Ok(o) if o == b"hello" => {}
                Ok(o) => failures.push(format!(
                    "split at {k}: got {:?}",
                    String::from_utf8_lossy(&o)
                )),
                Err(e) => failures.push(e),
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Oversized chunks are still refused: a chunk whose frame is
    /// incomplete but within the cap waits (`Ok`, nothing consumed), and a
    /// chunk whose declared size exceeds `total_len`'s `MAX_BODY` gate is
    /// rejected with `body too large`.
    #[test]
    fn oversized_chunk_still_refused() {
        // "A\r\n" declares a 10-byte chunk; at 12 bytes pending the frame
        // (needs 15) is incomplete → wait, no error, no output.
        let mut pending: Vec<u8> = b"A\r\n".to_vec();
        pending.extend_from_slice(b"012345678"); // 12 bytes pending
        let mut out = Vec::new();
        let mut consumed = 0usize;
        dechunk_step(&pending, &mut consumed, &mut |d| out.extend_from_slice(d))
            .unwrap_or_else(|e| panic!("incomplete 10-byte chunk must wait, got: {e}"));
        assert_eq!((consumed, out.as_slice()), (0usize, &b""[..]));

        // 0x1000000 = 16 MiB > MAX_BODY (8 MiB): total_len refuses.
        let oversized: &[u8] = b"1000000\r\n";
        let mut consumed = 0usize;
        match dechunk_step(oversized, &mut consumed, &mut |_| {}) {
            Err(PiiError::Invalid(m)) if m.contains("too large") => {}
            other => panic!("oversized chunk must be refused, got: {other:?}"),
        }
    }

    /// Trailer chunk: the terminator plus its trailer lines are consumed
    /// only once fully present; a mid-trailer split waits, then completes
    /// with `pos` landing after the final CRLF.
    #[test]
    fn trailer_chunk_waits_then_completes() {
        let body = b"5\r\nhello\r\n0\r\nX-T: v\r\n\r\n";
        let k = 17; // mid-trailer: "X-T:" present, " v\r\n\r\n" missing
        let mut pending: Vec<u8> = Vec::new();
        let mut out = Vec::new();
        for part in [&body[..k], &body[k..]] {
            pending.extend_from_slice(part);
            let mut consumed = 0usize;
            dechunk_step(&pending, &mut consumed, &mut |d| out.extend_from_slice(d))
                .unwrap_or_else(|e| panic!("trailer split at {k}: {e}"));
            assert!(
                consumed <= pending.len(),
                "trailer split at {k}: consumed {consumed} > pending {}",
                pending.len()
            );
            if consumed == 10 {
                // First step: decoded the data chunk but the terminator is
                // incomplete — R1.3 says it must wait, pos before "0".
                assert_eq!(&pending[..consumed], &b"5\r\nhello\r\n"[..]);
            }
            pending.drain(..consumed);
        }
        assert_eq!(out, b"hello");
        // Final step: the whole trailer sequence was consumed — pos landed
        // after the final CRLF.
        let mut consumed = 0usize;
        dechunk_step(&pending, &mut consumed, &mut |_| {})
            .unwrap_or_else(|e| panic!("trailer complete: {e}"));
        assert_eq!(consumed, pending.len());
    }
}

//! Incremental chunked-encoding decoding helpers and the deadline-bounded
//! read loop for the HTTP client side.

use pii_abi::PiiError;

use super::conn::Conn;

/// Incremental chunked-encoding decoder: consumes as many COMPLETE chunks
/// as `raw[..*pos]` contains, forwarding decoded body bytes through `out`.
/// State is fully derived from `raw` + `*pos`, so it can be called again
/// after more bytes arrive (resumable across socket reads).
pub(super) fn dechunk_step(
    raw: &[u8],
    pos: &mut usize,
    out: &mut dyn FnMut(&[u8]),
) -> Result<(), PiiError> {
    loop {
        // Need a full size line first.
        let Some(rel) = find_crlf(raw.get(*pos..).unwrap_or(&[])) else {
            return Ok(());
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
            // Terminating chunk: consume it and any trailing CRLF/trailers.
            *pos = data_start;
            while let Some(r) = find_crlf(raw.get(*pos..).unwrap_or(&[])) {
                let Some(trailer) = raw.get(*pos..*pos + r) else {
                    break;
                };
                *pos += r + 2;
                if trailer.is_empty() {
                    break;
                }
            }
            return Ok(());
        }
        if total_len(*pos, size)? > raw.len() {
            return Ok(()); // incomplete chunk: wait for more bytes
        }
        let chunk = raw
            .get(data_start..data_start + size)
            .ok_or_else(|| PiiError::Invalid("http: truncated chunk".into()))?;
        out(chunk);
        *pos = data_start + size;
        // Consume the CRLF after the chunk data.
        if raw.get(*pos) == Some(&b'\r') {
            *pos += 2;
        }
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

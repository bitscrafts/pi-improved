//! Hand-written HTTP/1.1 codec (F2.8). No dependency, total parsing: every
//! malformed input is an `Err`, never a panic, never an index panic.

use pii_abi::{HttpMethod, HttpRequest, HttpResponse, PiiError};

use crate::authorize::Target;

/// URL scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// Plaintext HTTP — the only scheme in Phase 3.
    Http,
}

const MAX_HEADER_BLOCK: usize = 64 * 1024;
const MAX_HEADERS: usize = 100;
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Parses an absolute `http://` URL into `(scheme, target, path-and-query)`.
///
/// # Errors
/// [`PiiError::Invalid`] for a malformed URL; `https` is refused explicitly
/// until Phase 3b (message names `tls` and `Phase 3b`).
pub fn parse_url(url: &str) -> Result<(Scheme, Target, String), PiiError> {
    let invalid = |m: &str| PiiError::Invalid(format!("http: {m}: '{url}'"));
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(invalid("missing scheme"));
    };
    match scheme {
        "http" => {}
        "https" => {
            return Err(PiiError::Invalid(
                "https requires the tls feature (Phase 3b)".into(),
            ));
        }
        other => return Err(invalid(&format!("unknown scheme '{other}'"))),
    }
    let (authority, path) = rest.find('/').map_or_else(
        || (rest, String::from("/")),
        |i| (&rest[..i], rest[i..].to_owned()),
    );
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port = p.parse::<u16>().map_err(|_| invalid("port must be u16"))?;
            (h.to_owned(), port)
        }
        None => (authority.to_owned(), 80),
    };
    if host.is_empty() {
        return Err(invalid("empty host"));
    }
    Ok((Scheme::Http, Target { host, port }, path))
}

/// Serialises an [`HttpRequest`] to wire bytes (F2.8). Caller headers may
/// not override `Host` or `Connection`. `path` comes from `parse_url`.
#[must_use]
pub fn render_request(req: &HttpRequest, host_header: &str, path: &str) -> Vec<u8> {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = write!(out, "{} {} HTTP/1.1\r\n", req.method, path);
    let _ = write!(out, "Host: {host_header}\r\n");
    out.push_str("User-Agent: pii/0.1\r\n");
    out.push_str("Connection: close\r\n");
    if !req.body.is_empty() {
        let _ = write!(out, "Content-Length: {}\r\n", req.body.len());
    }
    for (k, v) in &req.headers {
        let kl = k.to_ascii_lowercase();
        if kl == "host" || kl == "connection" || kl == "content-length" {
            continue;
        }
        let _ = write!(out, "{k}: {v}\r\n");
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(&req.body);
    bytes
}

/// One parsed header line pair.
fn parse_header_line(line: &str) -> Result<(String, String), PiiError> {
    let (k, v) = line
        .split_once(':')
        .ok_or_else(|| PiiError::Invalid("http: malformed header line".into()))?;
    Ok((k.trim().to_owned(), v.trim().to_owned()))
}

/// Reads one CRLF-terminated line from the byte buffer cursor.
fn read_line<'a>(buf: &'a [u8], pos: &mut usize) -> Result<&'a str, PiiError> {
    let start = *pos;
    while *pos < buf.len() {
        if buf.get(*pos) == Some(&b'\n') {
            let end = *pos;
            *pos += 1;
            let line = buf
                .get(start..end)
                .ok_or_else(|| PiiError::Invalid("http: truncated header block".into()))?;
            let line = if line.last() == Some(&b'\r') {
                line.get(..line.len() - 1).unwrap_or(line)
            } else {
                line
            };
            return std::str::from_utf8(line)
                .map_err(|_| PiiError::Invalid("http: non-utf8 header".into()));
        }
        *pos += 1;
        if *pos - start > MAX_HEADER_BLOCK {
            return Err(PiiError::Invalid("http: header block too large".into()));
        }
    }
    Err(PiiError::Invalid("http: truncated header block".into()))
}

/// Parses status line + headers, returning them with the offset of the
/// first body byte (F2.8). Shared by the buffered and the streaming
/// client paths so both accept exactly the same head grammar.
///
/// # Errors
/// [`PiiError::Invalid`] for a malformed or oversized head.
/// Parsed response head: status, header pairs, body start offset.
pub type ResponseHead = (u16, Vec<(String, String)>, usize);

/// Parses the status line + headers; returns the body start offset.
///
/// # Errors
/// [`PiiError::Invalid`] when the head is malformed.
pub fn parse_response_head(raw: &[u8]) -> Result<ResponseHead, PiiError> {
    let mut pos = 0usize;
    let status_line = read_line(raw, &mut pos)?;
    if !status_line.starts_with("HTTP/1.") {
        return Err(PiiError::Invalid(format!(
            "http: unsupported version in '{status_line}'"
        )));
    }
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| PiiError::Invalid("http: malformed status line".into()))?;
    let mut headers = Vec::new();
    loop {
        if headers.len() >= MAX_HEADERS {
            return Err(PiiError::Invalid("http: too many headers".into()));
        }
        let line = read_line(raw, &mut pos)?;
        if line.is_empty() {
            break;
        }
        headers.push(parse_header_line(line)?);
    }
    if pos > MAX_HEADER_BLOCK {
        return Err(PiiError::Invalid("http: header block too large".into()));
    }
    Ok((status, headers, pos))
}

/// Parses a full HTTP response (F2.8): status line, headers (≤100, block
/// ≤64 KiB), then a `Content-Length` or chunked body (≤8 MiB).
///
/// # Errors
/// [`PiiError::Invalid`] for every malformed or oversized input.
pub fn parse_response(raw: &[u8]) -> Result<HttpResponse, PiiError> {
    let (status, headers, mut pos) = parse_response_head(raw)?;
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    let body = if chunked {
        read_chunked(raw, &mut pos)?
    } else {
        let len: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        if len > MAX_BODY {
            return Err(PiiError::Invalid("http: body too large".into()));
        }
        let end = pos + len;
        if end > raw.len() {
            return Err(PiiError::Invalid("http: truncated body".into()));
        }
        raw.get(pos..end)
            .ok_or_else(|| PiiError::Invalid("http: truncated body".into()))?
            .to_vec()
    };
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn read_chunked(raw: &[u8], pos: &mut usize) -> Result<Vec<u8>, PiiError> {
    let mut body = Vec::new();
    loop {
        let size_line = read_line(raw, pos)?;
        let size_str = size_line.split(';').next().unwrap_or("0");
        let size = usize::try_from(
            u64::from_str_radix(size_str.trim(), 16)
                .map_err(|_| PiiError::Invalid("http: bad chunk size".into()))?,
        )
        .map_err(|_| PiiError::Invalid("http: body too large".into()))?;
        if body.len() + size > MAX_BODY {
            return Err(PiiError::Invalid("http: body too large".into()));
        }
        if size == 0 {
            // Terminating chunk: consume trailing CRLF (and trailers).
            loop {
                let line = read_line(raw, pos)?;
                if line.is_empty() {
                    break;
                }
            }
            return Ok(body);
        }
        let end = *pos + size;
        if end + 2 > raw.len() {
            return Err(PiiError::Invalid("http: truncated chunk".into()));
        }
        let chunk = raw
            .get(*pos..end)
            .ok_or_else(|| PiiError::Invalid("http: truncated chunk".into()))?;
        body.extend_from_slice(chunk);
        *pos = end;
        // CRLF after each chunk.
        if raw.get(*pos) == Some(&b'\r') {
            *pos += 2;
        }
    }
}

/// Renders a response for the built-in server side (F2.10).
#[must_use]
pub fn render_response(resp: &HttpResponse) -> Vec<u8> {
    use std::fmt::Write as _;
    let reason = match resp.status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    };
    let mut out = format!("HTTP/1.1 {} {reason}\r\nConnection: close\r\n", resp.status);
    for (k, v) in &resp.headers {
        if k.eq_ignore_ascii_case("connection") {
            continue;
        }
        let _ = write!(out, "{k}: {v}\r\n");
    }
    if !resp.body.is_empty()
        && !resp
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
    {
        let _ = write!(out, "Content-Length: {}\r\n", resp.body.len());
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    bytes.extend_from_slice(&resp.body);
    bytes
}

/// Whether a method has a defined wire name (all four do).
#[must_use]
pub fn method_name(m: HttpMethod) -> &'static str {
    match m {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Delete => "DELETE",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{parse_response, parse_url};

    #[test]
    fn url_forms() {
        let (s, t, p) = parse_url("http://h:8080/a/b?q=1").unwrap();
        assert_eq!(s, super::Scheme::Http);
        assert_eq!(t.host, "h");
        assert_eq!(t.port, 8080);
        assert_eq!(p, "/a/b?q=1");
        assert_eq!(parse_url("http://h").unwrap().1.port, 80);
        let e = parse_url("https://api.telegram.org/x").unwrap_err();
        assert!(e.to_string().contains("tls"), "{e}");
        assert!(e.to_string().contains("Phase 3b"));
        assert!(parse_url("ftp://h/").is_err());
        assert!(parse_url("http://").is_err());
    }

    #[test]
    fn content_length_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"hi");
    }

    #[test]
    fn chunked_body() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n0\r\n\r\n";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.body, b"hi");
    }

    #[test]
    fn malformed_inputs_are_errors() {
        assert!(parse_response(b"HTTP/2 200\r\n\r\n").is_err());
        assert!(parse_response(b"HTTP/1.").is_err());
        let big = format!("HTTP/1.1 200\r\nX: y\r\n{}", "z".repeat(70_000));
        assert!(parse_response(big.as_bytes()).is_err());
        assert!(parse_response(b"HTTP/1.1 200\r\n\r\nxx").is_ok()); // no CL → empty body
    }
}

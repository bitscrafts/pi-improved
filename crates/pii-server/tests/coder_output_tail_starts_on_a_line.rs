//! Spec 265: a run's kept tail starts on a whole line. Spec 263 cuts the tail forward to the first newline after its raw
//! start, so a line-based reader never sees half a JSON line at the seam. These streams put the raw start mid-line (or
//! exactly on a line start) on purpose, so a missing cut cannot pass by the seam landing on a boundary.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::io::Cursor;

use pii_server::node_kinds::nodes::agent_claude::run_facts;
use pii_server::node_kinds::nodes::coder_runtime::drain_stdout;

/// The most output one stream keeps (spec 209's `OUTPUT_KEEP_BYTES`, private to the runtime).
const KEEP: usize = 256 * 1024;

/// The most of a stream's first bytes kept as the head (spec 263's `OUTPUT_HEAD_BYTES`, `pub(crate)`).
const HEAD: usize = 64 * 1024;

/// The marker's type, which every line-based reader skips.
const MARK: &str = "{\"type\":\"pii_output_gap\"";

/// The length of the marker line `OutputKeep::finish` writes for `dropped` bytes, with its newline.
fn marker_len(dropped: usize) -> usize {
    format!("{{\"type\":\"pii_output_gap\",\"dropped_bytes\":{dropped}}}\n").len()
}

/// One filler line. Every filler line has the same width, so the line grid is known.
fn filler(n: u64) -> String {
    format!("{{\"type\":\"assistant\",\"n\":\"{n:06}\"}}\n")
}

/// A first line of exactly `len` bytes, JSON, so the test chooses where the line grid starts.
fn prefix(len: usize) -> String {
    let base = "{\"type\":\"system\",\"pad\":\"\"}\n".len();
    assert!(len >= base, "a prefix is at least {base} bytes");
    format!(
        "{{\"type\":\"system\",\"pad\":\"{}\"}}\n",
        "p".repeat(len - base)
    )
}

/// A prefix line of `prefix_len` bytes, then `fillers` whole filler lines.
fn stream(prefix_len: usize, fillers: u64) -> Vec<u8> {
    let mut out = prefix(prefix_len).into_bytes();
    for n in 0..fillers {
        out.extend_from_slice(filler(n).as_bytes());
    }
    out
}

/// A `assistant` line of `len` bytes in all, with its newline.
fn long_line(len: usize) -> String {
    let base = "{\"type\":\"assistant\",\"text\":\"\"}\n".len();
    format!(
        "{{\"type\":\"assistant\",\"text\":\"{}\"}}\n",
        "a".repeat(len - base)
    )
}

/// A `result` line of about `pad` bytes more than its fields, with its newline. It carries `is_error`, `turns`, `cost`.
fn result_line(pad: usize) -> String {
    format!(
        "{{\"type\":\"result\",\"subtype\":\"error\",\"is_error\":true,\"api_error_status\":401,\"result\":\"LAST\",\"num_turns\":9,\"total_cost_usd\":0.25,\"session_id\":\"sess-265\",\"pad\":\"{}\"}}\n",
        "r".repeat(pad)
    )
}

/// Where the keeper cuts a stream of `bytes`, from the real bounds: the head is cut back to its last newline, the tail
/// is the last `budget` bytes, and the raw tail start is `total - budget`.
struct Layout {
    total: usize,
    keep: usize,
    raw: usize,
    /// Every offset where a line of the stream starts.
    starts: Vec<usize>,
}

fn layout(bytes: &[u8]) -> Layout {
    let total = bytes.len();
    let keep = bytes[..HEAD]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(HEAD, |i| i + 1);
    let budget = KEEP - keep - marker_len(total);
    let mut starts = vec![0];
    starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter(|&(i, &b)| b == b'\n' && i + 1 < total)
            .map(|(i, _)| i + 1),
    );
    Layout {
        total,
        keep,
        raw: total - budget,
        starts,
    }
}

/// A stream whose raw tail start is mid-line, or exactly on a line start. The prefix length moves the line grid against
/// the cut, and a run of `width` lengths reaches each case.
fn stream_with_seam(on_a_line_start: bool) -> (Vec<u8>, Layout) {
    let width = filler(0).len();
    for len in 64..64 + width {
        let bytes = stream(len, 20_000);
        let l = layout(&bytes);
        if l.starts.contains(&l.raw) == on_a_line_start {
            return (bytes, l);
        }
    }
    panic!("no prefix length in a row of {width} puts the seam where asked");
}

/// The text after the marker line of a kept output.
fn tail_of(out: &str) -> &str {
    let start = out.find(MARK).expect("a marker line");
    let end = start + out[start..].find('\n').expect("the marker ends its line") + 1;
    &out[end..]
}

/// The text before the marker line of a kept output.
fn head_of(out: &str) -> &str {
    &out[..out.find(MARK).expect("a marker line")]
}

/// The `dropped_bytes` the marker line states.
fn dropped_of(out: &str) -> usize {
    let start = out.find(MARK).expect("a marker line");
    let line = &out[start..start + out[start..].find('\n').expect("the marker ends its line")];
    let json: serde_json::Value = serde_json::from_str(line).unwrap();
    usize::try_from(json["dropped_bytes"].as_u64().unwrap()).unwrap()
}

/// The whole lines of `bytes`, without their newlines.
fn lines_of(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The whole line of `bytes` that starts at `start`, without its newline.
fn line_at(bytes: &[u8], start: usize) -> &str {
    let rest = &bytes[start..];
    let end = rest.iter().position(|&b| b == b'\n').unwrap();
    std::str::from_utf8(&rest[..end]).unwrap()
}

#[test]
fn the_tail_never_starts_mid_line() {
    let (bytes, l) = stream_with_seam(false);
    assert!(
        !l.starts.contains(&l.raw),
        "the layout puts the raw tail start mid-line"
    );
    let out = drain_stdout(Cursor::new(bytes.clone()));
    let first = tail_of(&out).lines().next().expect("a tail");
    assert!(
        lines_of(&bytes).iter().any(|line| line == first),
        "the first tail line is a whole stream line: {first:?}"
    );
}

#[test]
fn every_line_after_the_marker_is_whole() {
    let (bytes, l) = stream_with_seam(false);
    assert!(
        !l.starts.contains(&l.raw),
        "the layout puts the raw tail start mid-line"
    );
    let out = drain_stdout(Cursor::new(bytes));
    let tail = tail_of(&out);
    assert!(!tail.is_empty(), "a tail is kept");
    for line in tail.lines() {
        assert!(
            serde_json::from_str::<serde_json::Value>(line).is_ok(),
            "a fragment after the marker: {line:?}"
        );
    }
}

#[test]
fn a_seam_inside_the_result_line_still_yields_the_result() {
    // The raw start falls inside the long line just before the result line, never inside the result line itself: the
    // cut drains through that long line's newline and the result line survives whole. Two cases: the raw start in the
    // middle of the long line, and near its start.
    for (long_len, near_start) in [(400_000_usize, false), (194_000_usize, true)] {
        let mut bytes = stream(64, 3_000);
        let long_start = bytes.len();
        bytes.extend_from_slice(long_line(long_len).as_bytes());
        bytes.extend_from_slice(result_line(3_000).as_bytes());
        let l = layout(&bytes);
        assert!(
            l.raw > long_start && l.raw < long_start + long_len,
            "the raw tail start is inside the long line"
        );
        if near_start {
            assert!(
                l.raw - long_start < 4_096,
                "the raw tail start is near the long line's start"
            );
        }
        let out = drain_stdout(Cursor::new(bytes));
        let facts = run_facts(&out);
        assert_eq!(facts.is_error, Some(true), "long line {long_len}");
        assert_eq!(facts.api_error_status, Some(401));
        assert_eq!(facts.result_text.as_deref(), Some("LAST"));
        assert_eq!(facts.turns, Some(9));
        assert_eq!(facts.cost_usd, Some(0.25));
    }
}

#[test]
fn the_dropped_count_matches_the_cut_tail() {
    let (bytes, l) = stream_with_seam(false);
    assert!(
        !l.starts.contains(&l.raw),
        "the layout puts the raw tail start mid-line"
    );
    let out = drain_stdout(Cursor::new(bytes));
    // The cut drains through the first newline after the raw start, so the kept tail starts at the next line start.
    let first_kept = *l.starts.iter().find(|&&s| s > l.raw).unwrap();
    let dropped = dropped_of(&out);
    assert_eq!(
        dropped,
        first_kept - l.keep,
        "the count runs to the first kept line"
    );
    assert_eq!(
        head_of(&out).len() + tail_of(&out).len() + dropped,
        l.total,
        "head, tail and dropped add up to the stream"
    );
}

#[test]
fn a_seam_exactly_on_a_line_start_is_not_cut_further() {
    let (bytes, l) = stream_with_seam(true);
    assert!(
        l.starts.contains(&l.raw),
        "the layout puts the raw tail start on a line start"
    );
    let out = drain_stdout(Cursor::new(bytes.clone()));
    // Measured: the cut drains through the first newline at or after the raw start, and that newline ends the line which
    // starts there. So the line at the raw start is lost by design (at most one line), and the first kept line is the
    // one after it.
    let next = *l.starts.iter().find(|&&s| s > l.raw).unwrap();
    assert_eq!(
        tail_of(&out).lines().next(),
        Some(line_at(&bytes, next)),
        "the first kept line is the one after the line at the raw start"
    );
    assert!(
        !out.contains(line_at(&bytes, l.raw)),
        "the line at the raw start is the one lost by design"
    );
}

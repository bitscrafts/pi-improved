//! Whether an agent run hit the account's spend limit, read from the CLI's own words in its captured output (spec 237).
//!
//! Observability only: the signal is journalled on `coder.exited`, and no verdict, park or retry reads it. The markers
//! are the shapes `scripts/autonomy/window.sh` already trusts. Text that merely mentions quota or a rate limit does not
//! match, because only the CLI's phrasing counts.
//!
//! A claude or pi run prints a JSON stream, and that stream carries everything the agent READ: a spec, a script or a
//! test fixture that quotes the CLI's phrase is echoed in a `tool_result` line. Measured 2026-10-26: eight cycles in a
//! row were reported `quota_exhausted=yes` because the worker had read a file quoting `resets 8:20pm`, and the loop
//! re-dispatched each time. So on a JSON stream only a line that carries the refusal as STRUCTURE counts: a top-level
//! `api_error_status` of 429, an error type of `rate_limit_error`, or a `result` line with `is_error` true whose own
//! `result` text holds a marker. Output that is not a JSON stream (plain text) is scanned whole, as before; stderr is the
//! CLI's own and is scanned whole.

/// A spend-limit message from an agent run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaSignal {
    /// The reset time the CLI printed, `resets H:MMam|pm (UTC)`; empty when it printed none.
    pub reset_text: String,
}

/// The CLI's own spend-limit phrasing, lowercase. Matched against the lowercased output and stderr.
const MARKERS: [&str; 5] = [
    "hit your session limit",
    "hit your weekly limit",
    "hit your monthly limit",
    "\"api_error_status\":429",
    "\"rate_limit_error\"",
];

/// The quota signal in an agent run's stdout or stderr, `None` when neither carries one.
#[must_use]
pub fn quota_signal(output: &str, stderr: &str) -> Option<QuotaSignal> {
    output_refusal(output)
        .or_else(|| is_limit(stderr).then(|| stderr.to_owned()))
        .map(|text| QuotaSignal {
            reset_text: reset_text(&text),
        })
}

/// The refusal in stdout: the structured line when stdout is a JSON stream, the whole text when it is plain output
/// (spec 237's rule, kept for a CLI that prints its refusal as bytes).
fn output_refusal(output: &str) -> Option<String> {
    let objects: Vec<serde_json::Map<String, serde_json::Value>> = output
        .lines()
        .filter_map(|line| match serde_json::from_str(line) {
            Ok(serde_json::Value::Object(obj)) => Some(obj),
            _ => None,
        })
        .collect();
    if objects.is_empty() {
        return is_limit(output).then(|| output.to_owned());
    }
    objects.iter().find_map(refusal_text)
}

/// The refusal a single JSON line carries as structure, as text to read the reset time from.
fn refusal_text(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    let type_is = |v: Option<&serde_json::Value>| {
        v.and_then(serde_json::Value::as_str) == Some("rate_limit_error")
    };
    let status_429 = obj
        .get("api_error_status")
        .and_then(serde_json::Value::as_i64)
        == Some(429);
    let rate_error =
        type_is(obj.get("type")) || type_is(obj.get("error").and_then(|e| e.get("type")));
    let result = obj
        .get("result")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let is_error = obj.get("is_error").and_then(serde_json::Value::as_bool) == Some(true);
    let result_marker = obj.get("type").and_then(serde_json::Value::as_str) == Some("result")
        && is_error
        && is_limit(result);
    (status_429 || rate_error || result_marker).then(|| result.to_owned())
}

/// Whether `text` carries one of the CLI's spend-limit markers, in any letter case.
fn is_limit(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    MARKERS.iter().any(|marker| lower.contains(marker))
}

/// The first `resets H:MMam|pm (UTC)` in `text`, as written in `text`; empty when there is none.
fn reset_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    lower
        .match_indices("resets ")
        .find_map(|(at, word)| {
            let start = at + word.len();
            let len = clock_len(lower.get(start..)?)?;
            Some(at..start + len)
        })
        .and_then(|range| text.get(range).map(str::to_owned))
        .unwrap_or_default()
}

/// The length of a leading `H:MMam|pm (utc)` in `s`, when `s` starts with one.
fn clock_len(s: &str) -> Option<usize> {
    let hours = s.bytes().take(2).take_while(u8::is_ascii_digit).count();
    let tail = s.get(hours..)?.strip_prefix(':')?;
    let minutes = tail.get(..2)?;
    let suffix = tail.get(2..)?;
    let meridiem = suffix.get(..2)?;
    let zone = suffix.get(2..)?;
    let well_formed = hours >= 1
        && minutes.bytes().all(|c| c.is_ascii_digit())
        && matches!(meridiem, "am" | "pm")
        && zone.starts_with(" (utc)");
    well_formed.then_some(hours + ":MM".len() + "am (utc)".len())
}

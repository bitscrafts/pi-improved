//! Whether an agent run hit the account's spend limit, read from the CLI's own words in its captured output (spec 237).
//!
//! Observability only: the signal is journalled on `coder.exited`, and no verdict, park or retry reads it. The markers
//! are the shapes `scripts/autonomy/window.sh` already trusts. Text that merely mentions quota or a rate limit does not
//! match, because only the CLI's phrasing counts.

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
    [output, stderr]
        .into_iter()
        .find(|text| is_limit(text))
        .map(|text| QuotaSignal {
            reset_text: reset_text(text),
        })
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

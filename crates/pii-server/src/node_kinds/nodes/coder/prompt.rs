//! The prompt a coder node hands its agent: the brief, plus one sentence naming the spec the agent implements (spec 246).

/// The brief with the spec path appended as one paragraph.
///
/// The brief comes first, unchanged. An empty spec path, or a brief that already names the spec, leaves the brief as it is.
#[must_use]
pub fn brief_with_spec(brief: &str, spec_path: &str) -> String {
    let spec = spec_path.trim();
    if spec.is_empty() || brief.contains(spec) {
        return brief.to_owned();
    }
    format!(
        "{brief}\n\nThe spec you implement is `{spec}` (relative to your working directory). Read it first: it is the contract."
    )
}

/// The prompt with credentials masked, for the JOURNAL only (never for the model).
///
/// Two requirements meet here and neither may yield: spec 386 R4 says "what was it told" is answerable
/// from the record, and `coder_secret_never_journalled` says a journal never carries a credential. The
/// text is therefore journalled REDACTED — a reader sees the instruction, a disk never sees the key.
///
/// The patterns are the shapes a credential takes in a brief: a `sk-...`-style token, or a value assigned
/// to a key-ish word (`--api-key X`, `token=X`, `password: X`). Under-redacting is a leak; over-redacting
/// costs a reader some words, so the net is drawn generously.
#[must_use]
pub fn redacted(prompt: &str) -> String {
    let mut out = String::with_capacity(prompt.len());
    for line in prompt.split_inclusive('\n') {
        out.push_str(&redact_line(line));
    }
    out
}

fn redact_line(line: &str) -> String {
    let mut s = line.to_owned();
    // (a) a provider token: sk- followed by at least eight token characters
    let mut at = 0;
    while let Some(i) = s[at..].find("sk-") {
        let start = at + i;
        let tail: String = s[start..]
            .chars()
            .skip(3)
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if tail.len() >= 8 {
            s.replace_range(start..start + 3 + tail.len(), "sk-REDACTED");
        }
        at = start + 3;
        if at >= s.len() {
            break;
        }
    }
    // (b) key = value, where the key names a credential
    for marker in [
        "api-key", "api_key", "apikey", "token", "secret", "password",
    ] {
        let lower = s.to_lowercase();
        let mut from = 0;
        while let Some(i) = lower[from..].find(marker) {
            let at = from + i + marker.len();
            let rest = s.get(at..).unwrap_or("");
            let sep_len = rest
                .chars()
                .take_while(|c| *c == ' ' || *c == '=' || *c == ':' || *c == '"')
                .count();
            if sep_len > 0 {
                let value_start = at
                    + rest
                        .char_indices()
                        .take(sep_len)
                        .last()
                        .map_or(0, |(i, c)| i + c.len_utf8());
                let value: String = s
                    .get(value_start..)
                    .unwrap_or("")
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
                    .collect();
                if value.len() >= 4 {
                    s.replace_range(value_start..value_start + value.len(), "REDACTED");
                }
            }
            from = at;
            if from >= s.len() {
                break;
            }
        }
    }
    s
}

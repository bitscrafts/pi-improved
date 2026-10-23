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

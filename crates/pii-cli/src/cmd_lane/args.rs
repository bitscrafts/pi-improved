//! The script's own arguments, parsed the way the shell parses them (spec 317 R2). Pure.

/// `--resume`: absent, given without a value (an error), or a session id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resume {
    None,
    MissingValue,
    Sid(String),
}

/// The parsed script arguments: the spec number, `--resume`, the extra brief, `--dry-run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Args {
    pub(crate) spec: Option<String>,
    pub(crate) resume: Resume,
    /// The last non-option word; an empty word counts as none.
    pub(crate) brief: Option<String>,
    pub(crate) dry: bool,
}

impl Args {
    /// Parses `<N> [--resume <sid>] [<extra-brief>] [--dry-run]`. Never fails: the
    /// caller reports `Resume::MissingValue` in the script's own order.
    pub(crate) fn parse(words: &[String]) -> Self {
        let mut a = Self {
            spec: words.first().filter(|s| !s.is_empty()).cloned(),
            resume: Resume::None,
            brief: None,
            dry: false,
        };
        let mut it = words.iter().skip(1);
        while let Some(w) = it.next() {
            match w.as_str() {
                "--resume" => {
                    let value = it.next().filter(|v| !v.is_empty());
                    a.resume = match (value, &a.resume) {
                        (_, Resume::MissingValue) | (None, _) => Resume::MissingValue,
                        (Some(v), _) => Resume::Sid(v.clone()),
                    };
                }
                "--dry-run" => a.dry = true,
                other => a.brief = Some(other.to_owned()),
            }
        }
        a
    }
}

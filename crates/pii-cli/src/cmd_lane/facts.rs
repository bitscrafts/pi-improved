//! The journal's fields, read the way the shell's `grep -o` and `sed` read them (spec 319 R5). Pure.

/// `grep -o "\"key\":\"\?[^,\"}]*" | head -1 | sed -E 's/^"[a-z_]*":"?//'`: the value of a bare or quoted field.
pub(crate) fn fact(json: &str, key: &str) -> String {
    let pat = format!("\"{key}\":");
    let Some(at) = json.find(&pat) else {
        return String::new();
    };
    let rest = &json[at + pat.len()..];
    let rest = rest.strip_prefix('"').unwrap_or(rest);
    rest.chars()
        .take_while(|c| !matches!(c, ',' | '"' | '}'))
        .collect()
}

/// `grep -o '"key":"[^"]*"' | cut -d'"' -f4 | head -1`: the content of the first quoted field.
pub(crate) fn quoted(json: &str, key: &str) -> String {
    let pat = format!("\"{key}\":\"");
    json.find(&pat)
        .and_then(|at| {
            let rest = &json[at + pat.len()..];
            rest.find('"').map(|end| rest[..end].to_owned())
        })
        .unwrap_or_default()
}

/// `grep -oE '[0-9]{1,2}:[0-9]{2}(am|pm)' | head -1`: the first clock time in a reset text.
pub(crate) fn reset_time(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    // `c` is the colon: the hour is one digit before it, or two (the greedy form, tried first).
    let tail_ok = |c: usize| {
        b.get(c) == Some(&b':')
            && digit(c + 1)
            && digit(c + 2)
            && b.get(c + 3..c + 5)
                .is_some_and(|m| m == b"am" || m == b"pm")
    };
    (0..b.len()).find_map(|i| {
        [i + 2, i + 1].into_iter().find_map(|c| {
            let hour_ok = if c == i + 2 {
                digit(i) && digit(i + 1)
            } else {
                digit(i)
            };
            (hour_ok && tail_ok(c)).then(|| text[i..c + 5].to_owned())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_reads_bare_and_quoted_values() {
        let j = r#"{"rc":"0","tree_changed":true,"quota_exhausted":"yes"}"#;
        assert_eq!(fact(j, "rc"), "0");
        assert_eq!(fact(j, "tree_changed"), "true");
        assert_eq!(fact(j, "quota_exhausted"), "yes");
        assert_eq!(fact(j, "missing"), "");
    }

    #[test]
    fn reset_time_takes_the_first_clock_time() {
        assert_eq!(reset_time("resets 5:30pm soon").as_deref(), Some("5:30pm"));
        assert_eq!(reset_time("at 123:30am").as_deref(), Some("23:30am"));
        assert_eq!(reset_time("no time"), None);
    }
}

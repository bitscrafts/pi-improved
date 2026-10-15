//! `pii deploy-card {render|approve|refuse} --card <file.json>` (spec 152): the
//! one permission that stays human. The card file is the promotion's own
//! record (incident, change, proof, gate, rollback rehearsal); a card missing
//! any of the five fields is REFUSED with the field named and no approval
//! path printed. Approval is bound to the revision the card names. Every
//! decision appends one line to the operator records (`PII_ACTS`, default
//! `_tmp/autonomy/operator-acts.log`, the file spec 153's metric counts).
//! Headless: no prompt, no interactive input.

use std::io::Write as _;
use std::process::ExitCode;

use serde_json::Value;

use crate::cmd_ctl::next_str;

/// The five fields, each with the sub-keys that must be present and non-empty.
const FIELDS: [(&str, &[&str]); 5] = [
    ("incident", &["id", "fingerprint", "count"]),
    ("change", &["branch", "revision", "diff_stat"]),
    ("proof", &["red_on_base", "green_on_branch"]),
    ("gate", &["verdict"]),
    ("rollback", &["rehearsal"]),
];

fn present(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        _ => true,
    }
}

/// Fields that cannot be sourced from the card, as `field` or `field (key)`.
fn missing(card: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for (field, keys) in FIELDS {
        match card.get(field) {
            None => out.push(field.to_owned()),
            Some(obj) => {
                for k in keys {
                    if !obj.get(k).is_some_and(present) {
                        out.push(format!("{field} ({k})"));
                    }
                }
            }
        }
    }
    out
}

fn text(card: &Value, field: &str, key: &str) -> String {
    match card.get(field).and_then(|o| o.get(key)) {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

/// Fields present out of five (a field counts only when all its keys do).
fn fields_present(card: &Value) -> usize {
    FIELDS
        .iter()
        .filter(|(f, keys)| {
            card.get(*f)
                .is_some_and(|o| keys.iter().all(|k| o.get(k).is_some_and(present)))
        })
        .count()
}

/// `YYYY-MM-DDTHH:MM:SSZ` for now, without a date dependency.
// reason: an operator record is stamped with the wall clock; there is no replay here.
#[allow(clippy::disallowed_methods)]
fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (i64::try_from(secs / 86_400).unwrap_or(0), secs % 86_400);
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// One line in the operator records; a log that cannot be written never fails the act.
fn record(card: &Value, answer: &str, detail: &str) {
    let path = std::env::var_os("PII_ACTS").map_or_else(
        || "_tmp/autonomy/operator-acts.log".into(),
        std::path::PathBuf::from,
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let line = format!(
        "{} | human | deploy-card | incident={} rev={} fields={}/5 {answer} {detail}\n",
        now_utc(),
        text(card, "incident", "id"),
        text(card, "change", "revision"),
        fields_present(card),
    );
    let ok = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
    if ok.is_err() {
        let _ = writeln!(
            std::io::stderr(),
            "(operator-acts.log not writable; carrying on)"
        );
    }
}

fn refuse_incomplete(gaps: &[String]) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "deploy card REFUSED — no approval path offered");
    for g in gaps {
        let _ = writeln!(out, "missing field: {g}");
    }
}

fn render(card: &Value) {
    let mut o = std::io::stdout();
    let rev = text(card, "change", "revision");
    let _ = writeln!(
        o,
        "deploy card\n1 incident: {} fingerprint={} count={}\n2 change: branch={} revision={rev}\n{}\n3 proof, red on base:\n{}\n3 proof, green on branch:\n{}\n4 gate: {}\n5 rollback rehearsal:\n{}\n\napprove: pii deploy-card approve --card <file> --rev {rev}\nrefuse:  pii deploy-card refuse --card <file> [--reason <text>]",
        text(card, "incident", "id"),
        text(card, "incident", "fingerprint"),
        text(card, "incident", "count"),
        text(card, "change", "branch"),
        text(card, "change", "diff_stat"),
        text(card, "proof", "red_on_base"),
        text(card, "proof", "green_on_branch"),
        text(card, "gate", "verdict"),
        text(card, "rollback", "rehearsal"),
    );
}

/// `pii deploy-card {render|approve|refuse} --card <file> [--rev R] [--reason T]`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_deploy_card(args: &mut std::env::ArgsOs) -> ExitCode {
    let (mut sub, mut card_path, mut rev, mut reason) = (None, None, None, None);
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--card" => card_path = next_str(args),
            "--rev" => rev = next_str(args),
            "--reason" => reason = next_str(args),
            s if sub.is_none() && !s.starts_with("--") => sub = Some(s.to_owned()),
            _ => sub = Some(String::new()),
        }
    }
    let (Some(sub), Some(card_path)) = (sub, card_path) else {
        return usage();
    };
    let card: Value = match std::fs::read_to_string(&card_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(v) => v,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "error: card {card_path}: {e}");
            return ExitCode::from(2);
        }
    };
    let gaps = missing(&card);
    match sub.as_str() {
        "render" => {
            if gaps.is_empty() {
                render(&card);
                ExitCode::SUCCESS
            } else {
                refuse_incomplete(&gaps);
                ExitCode::from(3)
            }
        }
        "approve" => {
            let Some(found) = rev else { return usage() };
            if !gaps.is_empty() {
                record(&card, "refused", &format!("missing={}", gaps.join(",")));
                refuse_incomplete(&gaps);
                return ExitCode::from(3);
            }
            let expected = text(&card, "change", "revision");
            if found != expected {
                record(
                    &card,
                    "refused",
                    &format!("expected={expected} found={found}"),
                );
                let _ = writeln!(
                    std::io::stdout(),
                    "approval REFUSED — revision mismatch: expected={expected} found={found}"
                );
                return ExitCode::from(3);
            }
            record(&card, "approved", "");
            let _ = writeln!(std::io::stdout(), "APPROVED revision {expected}");
            ExitCode::SUCCESS
        }
        "refuse" => {
            record(
                &card,
                "refused",
                &format!("reason={}", reason.unwrap_or_default()),
            );
            let _ = writeln!(std::io::stdout(), "card refused by the person; recorded");
            ExitCode::SUCCESS
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    let _ = writeln!(
        std::io::stderr(),
        "usage: pii deploy-card {{render|approve --rev <rev>|refuse [--reason <text>]}} --card <file.json>"
    );
    ExitCode::from(2)
}

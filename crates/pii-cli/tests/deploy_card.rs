//! Spec 152: the deploy card — five sourced fields, or it is refused; approval
//! is bound to the revision; every decision leaves one operator line.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// reason: the test runs the built `pii` binary and reads its output.
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::Output;

use serde_json::{Value, json};

const FIELDS: [&str; 5] = ["incident", "change", "proof", "gate", "rollback"];

fn full_card() -> Value {
    json!({
        "incident": {"id": "inc-1", "fingerprint": "abc123", "count": 4},
        "change": {"branch": "fix/abc123", "revision": "rev-A", "diff_stat": "1 file changed, 2 insertions(+)"},
        "proof": {"red_on_base": "test x ... FAILED", "green_on_branch": "test x ... ok"},
        "gate": {"verdict": "ALL PASS"},
        "rollback": {"rehearsal": "RESTORED old-pii\nDRY-RUN OK"}
    })
}

fn pii(args: &[&str], acts: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_pii"))
        .arg("deploy-card")
        .args(args)
        .env("PII_ACTS", acts)
        .output()
        .unwrap()
}

fn write_card(dir: &Path, name: &str, card: &Value) -> String {
    let p = dir.join(name);
    std::fs::write(&p, card.to_string()).unwrap();
    p.to_str().unwrap().to_owned()
}

fn acts_lines(acts: &Path) -> Vec<String> {
    std::fs::read_to_string(acts)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_complete_card_renders_and_offers_approval() {
    let d = tempfile::tempdir().unwrap();
    let acts = d.path().join("acts.log");
    let card = write_card(d.path(), "c.json", &full_card());
    let out = pii(&["render", "--card", &card], &acts);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("pii deploy-card approve"), "{text}");
    assert!(text.contains("rev-A") && text.contains("abc123"), "{text}");
}

#[test]
fn a_card_missing_any_field_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let acts = d.path().join("acts.log");
    for f in FIELDS {
        let mut c = full_card();
        c.as_object_mut().unwrap().remove(f);
        let card = write_card(d.path(), &format!("{f}.json"), &c);
        let out = pii(&["render", "--card", &card], &acts);
        assert_ne!(out.status.code(), Some(0), "{f}: {out:?}");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(text.contains("REFUSED"), "{f}: {text}");
        assert!(text.contains(&format!("missing field: {f}")), "{f}: {text}");
        assert!(
            !text.contains("deploy-card approve"),
            "{f}: no approval path may be offered: {text}"
        );
        // approving the incomplete card is refused too
        let out = pii(&["approve", "--card", &card, "--rev", "rev-A"], &acts);
        assert_ne!(out.status.code(), Some(0), "{f}: approve must refuse");
    }
}

#[test]
fn approval_is_bound_to_the_revision() {
    let d = tempfile::tempdir().unwrap();
    let acts = d.path().join("acts.log");
    let card = write_card(d.path(), "c.json", &full_card());
    let out = pii(&["approve", "--card", &card, "--rev", "rev-B"], &acts);
    assert_ne!(out.status.code(), Some(0), "{out:?}");
    let lines = acts_lines(&acts);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let l = lines.first().unwrap();
    assert!(
        l.contains("expected=rev-A") && l.contains("found=rev-B") && l.contains("refused"),
        "{l}"
    );
    // and the matching revision is accepted
    let out = pii(&["approve", "--card", &card, "--rev", "rev-A"], &acts);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
}

#[test]
fn the_rollback_line_is_rehearsed_before_the_card() {
    let d = tempfile::tempdir().unwrap();
    let acts = d.path().join("acts.log");
    for bad in [
        json!({}),
        json!({"rehearsal": ""}),
        json!({"rehearsal": "  \n"}),
    ] {
        let mut c = full_card();
        c.as_object_mut().unwrap().insert("rollback".into(), bad);
        let card = write_card(d.path(), "c.json", &c);
        let out = pii(&["render", "--card", &card], &acts);
        assert_ne!(out.status.code(), Some(0), "{out:?}");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            text.contains("REFUSED") && text.contains("rollback"),
            "{text}"
        );
        assert!(!text.contains("deploy-card approve"), "{text}");
    }
}

#[test]
fn one_card_one_operator_line() {
    let d = tempfile::tempdir().unwrap();
    let acts = d.path().join("acts.log");
    let card = write_card(d.path(), "c.json", &full_card());
    let r = pii(&["render", "--card", &card], &acts);
    assert_eq!(r.status.code(), Some(0));
    let a = pii(&["approve", "--card", &card, "--rev", "rev-A"], &acts);
    assert_eq!(a.status.code(), Some(0), "{a:?}");
    let lines = acts_lines(&acts);
    assert_eq!(lines.len(), 1, "{lines:?}");
    let l = lines.first().unwrap();
    assert!(
        l.contains("| human |") && l.contains("deploy-card") && l.contains("inc-1"),
        "{l}"
    );
    assert!(l.contains("approved") && l.contains("fields=5/5"), "{l}");
}

#[test]
fn a_person_can_refuse_a_complete_card() {
    let d = tempfile::tempdir().unwrap();
    let acts = d.path().join("acts.log");
    let card = write_card(d.path(), "c.json", &full_card());
    let out = pii(&["refuse", "--card", &card, "--reason", "not today"], &acts);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let lines = acts_lines(&acts);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines.first().unwrap().contains("refused"), "{lines:?}");
}

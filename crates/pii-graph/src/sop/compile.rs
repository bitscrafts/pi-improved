//! The SOP → graph compiler. Pure text generation in a fixed order with no
//! maps and no clock, so the same SOP is the same bytes on every run.
//!
//! Per step `S` (every id below is derived from the step id):
//! `S.begin` (a `sop` node: checks `requires`, emits `sop.step.started`) →
//! the step's own node(s) → `S.end` (emits `sop.step.finished`) → the next
//! step's `.begin`. Every failure path ends in `S.fail`, a `sop` node that
//! emits `sop.step.failed` and fails the run naming the step.

use std::fmt::Write as _;

use super::{Sop, Step, StepKind};

/// `write!` into a `String` cannot fail; `.ok()` says so without a discarded
/// `must_use`.
macro_rules! wr {
    ($dst:expr, $($arg:tt)*) => {
        $dst.write_fmt(format_args!($($arg)*)).ok()
    };
}
macro_rules! wrln {
    ($dst:expr, $($arg:tt)*) => {
        $dst.write_fmt(format_args!("{}\n", format_args!($($arg)*))).ok()
    };
}

/// Compiles `sop` into graph TOML (the text `pii sop compile` prints).
#[must_use]
pub fn compile(sop: &Sop) -> String {
    let mut out = String::new();
    wrln!(
        out,
        "# Compiled from SOP {} ({}) by `pii sop compile` - generated, do not edit.",
        q(&sop.id),
        q(&sop.title)
    );
    wrln!(out, "name = {}", q(&format!("sop/{}", sop.id)));
    let entry = sop
        .steps
        .first()
        .map_or_else(String::new, |s| format!("{}.begin", s.id));
    wrln!(out, "entry = {}", q(&entry));
    wrln!(out, "caps = {}", list(&sop.caps));
    for (i, step) in sop.steps.iter().enumerate() {
        nodes(&mut out, sop, step);
        if let Some(next) = sop.steps.get(i + 1) {
            edge(
                &mut out,
                &format!("{}.end", step.id),
                "ok",
                &format!("{}.begin", next.id),
            );
        }
    }
    out
}

fn nodes(out: &mut String, sop: &Sop, s: &Step) {
    let id = &s.id;
    let mut begin = sop_node(sop, s, "begin", "begin");
    wrln!(begin, "requires = {}", list(&s.requires));
    // The Act step's instruction lives on the AGENT node (`system`), which is the node that reads it.
    // Its `begin` node used to carry the same text as `prompt` — a field no non-agent kind reads, and
    // spec 386 refuses a prompt on such a node (measured 2026-11-05: three sop_engine tests).
    out.push_str(&begin);
    let fail_from: Vec<(String, &str)> = match s.kind {
        StepKind::Act => act(out, sop, s),
        StepKind::Check => check(out, s),
        StepKind::Approve => approve(out, sop, s),
        StepKind::Notify => notify(out, s),
    };
    out.push_str(&sop_node(sop, s, "end", "end"));
    let mut fail = sop_node(sop, s, "fail", "fail");
    // One inbound edge is not a race (V9); several failure ports are.
    if fail_from.len() > 1 {
        wrln!(fail, "join = \"any\"");
    }
    if s.kind == StepKind::Approve {
        wrln!(fail, "cause = \"approval denied\"");
    }
    if !fail_from.is_empty() {
        out.push_str(&fail);
    }
    for (from, port) in fail_from {
        edge(out, &from, port, &format!("{id}.fail"));
    }
}

/// A `sop` node block for `op`, id `<step>.<suffix>`.
fn sop_node(sop: &Sop, s: &Step, op: &str, suffix: &str) -> String {
    format!(
        "\n[[node]]\nid = {}\nkind = \"sop\"\nop = {}\nsop = {}\nstep = {}\nstep_kind = {}\n",
        q(&format!("{}.{suffix}", s.id)),
        q(op),
        q(&sop.id),
        q(&s.id),
        q(s.kind.as_str())
    )
}

fn act(out: &mut String, sop: &Sop, s: &Step) -> Vec<(String, &'static str)> {
    let id = &s.id;
    wr!(
        out,
        "\n[[node]]\nid = {}\nkind = \"agent\"\nsystem = {}\ntools = {}\nmax_turns = {}\n",
        q(id),
        q(&format!(
            "You are executing step '{id}' of the procedure '{}' ({}). Do exactly this step and report the result.",
            sop.id, sop.title
        )),
        list(&s.tools),
        s.max_turns.unwrap_or(8)
    );
    edge(out, &format!("{id}.begin"), "ok", id);
    edge(out, id, "ok", &format!("{id}.end"));
    ["err", "budget", "fail"]
        .into_iter()
        .map(|p| (id.clone(), p))
        .collect()
}

fn check(out: &mut String, s: &Step) -> Vec<(String, &'static str)> {
    let id = &s.id;
    wr!(
        out,
        "\n[[node]]\nid = {}\nkind = \"tool\"\ntool = \"bash\"\nargs = {{ command = {} }}\n",
        q(id),
        q(s.command.as_deref().unwrap_or_default())
    );
    wr!(
        out,
        "\n[[node]]\nid = {}\nkind = \"route\"\non = \"$.payload.exit_code\"\nrules = [\n  {{ eq = 0, port = \"pass\" }},\n  {{ port = \"fail\" }},\n]\n",
        q(&format!("{id}.verdict"))
    );
    edge(out, &format!("{id}.begin"), "ok", id);
    edge(out, id, "ok", &format!("{id}.verdict"));
    edge(out, &format!("{id}.verdict"), "pass", &format!("{id}.end"));
    vec![(id.clone(), "err"), (format!("{id}.verdict"), "fail")]
}

fn approve(out: &mut String, sop: &Sop, s: &Step) -> Vec<(String, &'static str)> {
    let id = &s.id;
    out.push_str(&sop_node(sop, s, "await", "await"));
    wrln!(out, "gate = {}", q(id));
    wr!(
        out,
        "\n[[node]]\nid = {}\nkind = \"gate\"\npolicy = \"sop\"\nprompt = {}\n",
        q(id),
        q(&format!(
            "SOP '{}' step '{id}': {}",
            sop.id,
            s.prompt.as_deref().unwrap_or_default()
        ))
    );
    edge(out, &format!("{id}.begin"), "ok", &format!("{id}.await"));
    edge(out, &format!("{id}.await"), "ok", id);
    edge(out, id, "allow", &format!("{id}.end"));
    vec![(id.clone(), "deny")]
}

fn notify(out: &mut String, s: &Step) -> Vec<(String, &'static str)> {
    let id = &s.id;
    let default_path = format!("_tmp/sop-{}-notify.log", s.id);
    wr!(
        out,
        "\n[[node]]\nid = {}\nkind = \"emit\"\nmode = \"append\"\npath = {}\nvalue = {}\n",
        q(id),
        q(s.path.as_deref().unwrap_or(&default_path)),
        q(&format!("{}\n", s.message.as_deref().unwrap_or_default()))
    );
    edge(out, &format!("{id}.begin"), "ok", id);
    edge(out, id, "ok", &format!("{id}.end"));
    vec![(id.clone(), "err")]
}

fn edge(out: &mut String, from: &str, port: &str, to: &str) {
    wr!(
        out,
        "\n[[edge]]\nfrom = {}\nport = {}\nto = {}\nkind = \"data\"\n",
        q(from),
        q(port),
        q(to)
    );
}

/// A TOML basic string. JSON string escaping is a subset of TOML's, except
/// DEL, which TOML forbids raw.
fn q(s: &str) -> String {
    serde_json::to_string(s)
        .unwrap_or_else(|_| String::from("\"\""))
        .replace('\u{7f}', "\\u007f")
}

fn list(items: &[String]) -> String {
    let body: Vec<String> = items.iter().map(|i| q(i)).collect();
    format!("[{}]", body.join(", "))
}

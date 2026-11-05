//! SOP lint: one message per problem, in a stable order.

use std::collections::{BTreeMap, BTreeSet};

use super::{RawSop, RawStep, Sop, Step, StepKind, sorted_unique, valid_id};

fn kind_of(s: &str) -> Option<StepKind> {
    match s {
        "act" => Some(StepKind::Act),
        "check" => Some(StepKind::Check),
        "approve" => Some(StepKind::Approve),
        "notify" => Some(StepKind::Notify),
        _ => None,
    }
}

/// Every problem in `raw`; empty means the SOP is well-formed.
pub(super) fn check(raw: &RawSop) -> Vec<String> {
    let mut errs = Vec::new();
    match raw.id.as_deref() {
        None | Some("") => errs.push(String::from("sop has no id")),
        Some(id) if !valid_id(id) => errs.push(format!(
            "sop id '{id}' may only use letters, digits, '_' and '-'"
        )),
        Some(_) => {}
    }
    if raw.title.as_deref().is_none_or(str::is_empty) {
        errs.push(String::from("sop has no title"));
    }
    for cap in &raw.caps {
        if pii_core::CapKind::parse_kind(cap).is_err() {
            errs.push(format!("unknown capability '{cap}'"));
        }
    }
    if raw.step.is_empty() {
        errs.push(String::from("an sop needs at least one step"));
    }
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, s) in raw.step.iter().enumerate() {
        let n = i + 1;
        match s.id.as_deref() {
            None | Some("") => errs.push(format!("step #{n} has no id")),
            Some(id) if !valid_id(id) => errs.push(format!(
                "step #{n} id '{id}' may only use letters, digits, '_' and '-'"
            )),
            Some(id) => {
                if let Some(first) = seen.insert(id, n) {
                    errs.push(format!(
                        "duplicate step id '{id}' (steps #{first} and #{n})"
                    ));
                }
            }
        }
        step_fields(s, n, &mut errs);
    }
    requires(raw, &seen, &mut errs);
    errs
}

fn step_fields(s: &RawStep, n: usize, errs: &mut Vec<String>) {
    let who =
        s.id.as_deref()
            .filter(|i| !i.is_empty())
            .map_or_else(|| format!("step #{n}"), |i| format!("step '{i}'"));
    let Some(kind_name) = s.kind.as_deref() else {
        errs.push(format!("{who} has no kind"));
        return;
    };
    let Some(kind) = kind_of(kind_name) else {
        errs.push(format!(
            "{who} has unknown kind '{kind_name}' (act|check|approve|notify)"
        ));
        return;
    };
    let (field, value) = match kind {
        StepKind::Act | StepKind::Approve => ("prompt", &s.prompt),
        StepKind::Check => ("command", &s.command),
        StepKind::Notify => ("message", &s.message),
    };
    match value.as_deref() {
        None | Some("") => errs.push(format!(
            "{who} (kind '{kind_name}') needs a '{field}'"
        )),
        Some(v) if v.starts_with("$.") => errs.push(format!(
            "{who}: '{field}' must not start with '$.' (the graph would evaluate it as an expression)"
        )),
        Some(_) => {}
    }
    // Spec 397: an approval that may wait for ever is a run that holds its row for ever.
    match (kind, s.deadline_ms) {
        (StepKind::Approve, None | Some(0)) => errs.push(format!(
            "{who} (kind 'approve') needs a positive 'deadline_ms' (how long it may wait for a decision)"
        )),
        (StepKind::Approve, Some(_)) | (_, None) => {}
        (_, Some(_)) => errs.push(format!("{who}: only an 'approve' step takes a 'deadline_ms'")),
    }
    if let Some(p) = &s.path
        && p.starts_with("$.")
    {
        errs.push(format!("{who}: 'path' must not start with '$.'"));
    }
}

/// Unknown references, then cycles over (declaration order ∪ `requires`).
fn requires(raw: &RawSop, seen: &BTreeMap<&str, usize>, errs: &mut Vec<String>) {
    let mut deps: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut prev: Option<&str> = None;
    for s in &raw.step {
        let Some(id) = s.id.as_deref().filter(|i| !i.is_empty()) else {
            prev = None;
            continue;
        };
        let entry = deps.entry(id).or_default();
        if let Some(p) = prev {
            entry.push(p);
        }
        for r in &s.requires {
            if seen.contains_key(r.as_str()) {
                entry.push(r.as_str());
            } else {
                errs.push(format!("step '{id}' requires unknown step '{r}'"));
            }
        }
        prev = Some(id);
    }
    if let Some(cycle) = find_cycle(&deps) {
        errs.push(format!("dependency cycle: {}", cycle.join(" -> ")));
    }
}

/// The first cycle found by DFS in id order, as `a -> b -> a`.
fn find_cycle(deps: &BTreeMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    fn visit<'a>(
        n: &'a str,
        deps: &BTreeMap<&'a str, Vec<&'a str>>,
        stack: &mut Vec<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> Option<Vec<String>> {
        if let Some(pos) = stack.iter().position(|s| *s == n) {
            let mut cyc: Vec<String> = stack.iter().skip(pos).map(|s| (*s).to_owned()).collect();
            cyc.push(n.to_owned());
            return Some(cyc);
        }
        if done.contains(n) {
            return None;
        }
        stack.push(n);
        for d in deps.get(n).map_or(&[][..], Vec::as_slice) {
            if let Some(c) = visit(d, deps, stack, done) {
                return Some(c);
            }
        }
        stack.pop();
        done.insert(n);
        None
    }
    let mut done = BTreeSet::new();
    for n in deps.keys() {
        if let Some(c) = visit(n, deps, &mut Vec::new(), &mut done) {
            return Some(c);
        }
    }
    None
}

/// Builds the validated model; only called after [`check`] found nothing.
pub(super) fn build(raw: RawSop) -> Sop {
    let id = raw.id.unwrap_or_default();
    let steps = raw
        .step
        .into_iter()
        .map(|s| Step {
            kind: s.kind.as_deref().and_then(kind_of).unwrap_or(StepKind::Act),
            id: s.id.unwrap_or_default(),
            requires: s.requires,
            prompt: s.prompt,
            command: s.command,
            message: s.message,
            path: s.path,
            tools: s.tools,
            max_turns: s.max_turns,
            deadline_ms: s.deadline_ms,
        })
        .collect();
    Sop {
        title: raw.title.unwrap_or_default(),
        caps: sorted_unique(&raw.caps),
        id,
        steps,
    }
}

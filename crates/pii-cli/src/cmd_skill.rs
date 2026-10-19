//! `pii skill {list|show|explain|edit}` (spec 210): the audit surface for `DocKind::Skill` documents.
//! It reads and writes the memory store at `<dir>/memory.redb` and nothing else; a skill's pinned
//! script is hashed where it lies (relative to the current directory) and never executed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_core::Clock as _;
use pii_memory::doc::{
    Doc, DocKind, Layer, created_of, decode_kind, decode_text, layer_byte, layer_of_byte, rows_for,
};
use pii_store::MemStore;
use sha2::{Digest as _, Sha256};

use crate::cmd_ctl::next_str;

const USAGE: &str = "usage: pii skill list [--all] [--layer L1|L2|L3] [--dir D]\n       \
pii skill show <id|slug> [--dir D]\n       pii skill explain <id|slug> [--dir D]\n       \
pii skill edit <id> --file <path> [--note <text>] [--dir D]\n\
--dir defaults to the runtime dir; the pin path is read relative to the current directory.";

/// The section headers `explain` renders from the body, verbatim.
const SECTIONS: [&str; 3] = ["WHEN TO USE", "CONTRACT", "VERBS"];

#[derive(Default)]
struct Opts {
    dir: Option<PathBuf>,
    all: bool,
    layer: Option<u8>,
    file: Option<PathBuf>,
    note: Option<String>,
    positional: Vec<String>,
}

/// One skill document as the store holds it: the payload, plus its layer and concept tags.
#[derive(Clone)]
struct Skill {
    id: String,
    text: String,
    layer: u8,
    created_ms: u64,
    concepts: Vec<String>,
    archived: bool,
}

/// One `script: <path> sha256=<hex>` line from a skill body. An empty `sha256` never matches.
struct Pin {
    path: String,
    sha256: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PinState {
    Ok,
    Drift,
    Missing,
}

impl PinState {
    fn label(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Drift => "DRIFT",
            Self::Missing => "MISSING FILE",
        }
    }
}

/// `pii skill {list|show|explain|edit} ...`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_skill(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("list") => list(args),
        Some("show") => show(args, false),
        Some("explain") => show(args, true),
        Some("edit") => edit(args),
        _ => usage(),
    }
}

fn parse(args: &mut std::env::ArgsOs) -> Option<Opts> {
    let mut o = Opts::default();
    while let Some(a) = next_str(args) {
        match a.as_str() {
            "--dir" => o.dir = Some(PathBuf::from(next_str(args)?)),
            "--all" => o.all = true,
            "--layer" => o.layer = Some(layer_tag(&next_str(args)?)?),
            "--file" => o.file = Some(PathBuf::from(next_str(args)?)),
            "--note" => o.note = Some(next_str(args)?),
            s if !s.starts_with("--") => o.positional.push(s.to_owned()),
            _ => return None,
        }
    }
    Some(o)
}

fn layer_tag(name: &str) -> Option<u8> {
    match name.to_ascii_uppercase().as_str() {
        "L1" => Some(layer_byte(Layer::L1)),
        "L2" => Some(layer_byte(Layer::L2)),
        "L3" => Some(layer_byte(Layer::L3)),
        _ => None,
    }
}

fn layer_label(tag: u8) -> &'static str {
    match layer_of_byte(tag) {
        Layer::L1 => "L1",
        Layer::L2 => "L2",
        Layer::L3 => "L3",
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn fail(msg: &str) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::from(2)
}

/// Opens the store read-write, but only where one already exists: a mistyped `--dir` must not
/// quietly create an empty memory file.
fn open(o: &Opts) -> Result<MemStore, ExitCode> {
    let Some(dir) = o.dir.clone().or_else(|| pii_uds::runtime_dir("default")) else {
        return Err(fail("no store directory (use --dir)"));
    };
    let path = dir.join("memory.redb");
    if !path.exists() {
        return Err(fail(&format!("no memory store at {}", path.display())));
    }
    MemStore::open(&path).map_err(|e| fail(&e.to_string()))
}

fn current_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Every `script: <path> sha256=<hex>` line of a body, in order.
fn pins_of(body: &str) -> Vec<Pin> {
    body.lines()
        .filter_map(|line| {
            let mut parts = line.trim().strip_prefix("script:")?.split_whitespace();
            let path = parts.next()?.to_owned();
            let sha256 = parts
                .find_map(|t| t.strip_prefix("sha256="))
                .unwrap_or("")
                .to_owned();
            Some(Pin { path, sha256 })
        })
        .collect()
}

/// Hashes the pinned file on disk and compares it with the pin.
fn verify(root: &Path, pin: &Pin) -> PinState {
    let Ok(bytes) = std::fs::read(root.join(&pin.path)) else {
        return PinState::Missing;
    };
    if sha256_hex(&bytes) == pin.sha256 {
        PinState::Ok
    } else {
        PinState::Drift
    }
}

/// The one status a listing shows for a body: `none`, or the worst state among its pins.
fn pin_summary(root: &Path, pins: &[Pin]) -> &'static str {
    if pins.is_empty() {
        return "none";
    }
    let states: Vec<PinState> = pins.iter().map(|p| verify(root, p)).collect();
    if states.contains(&PinState::Missing) {
        PinState::Missing.label()
    } else if states.contains(&PinState::Drift) {
        PinState::Drift.label()
    } else {
        PinState::Ok.label()
    }
}

/// The slug is the first non-empty line after the `PII-SKILL` marker.
fn slug_of(text: &str) -> &str {
    let mut lines = text.lines();
    lines.by_ref().find(|l| l.trim() == "PII-SKILL");
    lines.map(str::trim).find(|l| !l.is_empty()).unwrap_or("-")
}

fn short_id(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

/// `concept -> doc ids` read back as `doc id -> concepts`. The store keeps edges per concept, so
/// one pass over them is the only way to ask a doc its concepts.
fn concepts_by_doc(store: &MemStore) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for concept in store.concepts().map_err(|e| e.to_string())? {
        for (doc, _) in store.edges(&concept).map_err(|e| e.to_string())? {
            out.entry(doc).or_default().push(concept.clone());
        }
    }
    Ok(out)
}

/// Builds a `Skill` from a stored payload; `None` when the payload is not a skill document.
fn skill_of(
    store: &MemStore,
    concepts: &BTreeMap<String, Vec<String>>,
    id: &str,
    archived: bool,
) -> Option<Skill> {
    let bytes = if archived {
        store.archived(id)
    } else {
        store.doc(id)
    };
    let raw = Some(bytes.ok().flatten()?);
    if decode_kind(raw.clone()) != DocKind::Skill {
        return None;
    }
    Some(Skill {
        id: id.to_owned(),
        text: decode_text(raw.clone()),
        layer: store.layer(id).ok().flatten().unwrap_or(0),
        created_ms: created_of(raw),
        concepts: concepts.get(id).cloned().unwrap_or_default(),
        archived,
    })
}

/// Every skill in the store; archived ones only when asked.
fn skills(store: &MemStore, with_archived: bool) -> Result<Vec<Skill>, String> {
    let concepts = concepts_by_doc(store)?;
    let mut out = Vec::new();
    let live = store.doc_ids().map_err(|e| e.to_string())?;
    out.extend(
        live.iter()
            .filter_map(|id| skill_of(store, &concepts, id, false)),
    );
    if with_archived {
        let gone = store.archived_ids().map_err(|e| e.to_string())?;
        out.extend(
            gone.iter()
                .filter_map(|id| skill_of(store, &concepts, id, true)),
        );
    }
    Ok(out)
}

/// `pii skill list [--all] [--layer L] [--dir D]`: one line per skill.
fn list(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    let store = match open(&o) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let found = match skills(&store, o.all) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    let root = current_root();
    for s in found
        .iter()
        .filter(|s| o.layer.is_none_or(|l| l == s.layer))
    {
        let concepts = if s.concepts.is_empty() {
            String::from("-")
        } else {
            s.concepts.join(",")
        };
        let pins = pins_of(&s.text);
        let archived = if s.archived { " · archived" } else { "" };
        println!(
            "{} · {} · {} · {} · last_used {} · pin {}{}",
            short_id(&s.id),
            slug_of(&s.text),
            layer_label(s.layer),
            concepts,
            s.created_ms,
            pin_summary(&root, &pins),
            archived,
        );
    }
    ExitCode::SUCCESS
}

/// Resolves `<id|slug>`: an exact id or slug wins; else a unique id prefix. Several candidates
/// are refused with the list, so an ambiguous name never picks one silently.
fn resolve(found: &[Skill], key: &str) -> Result<Skill, ExitCode> {
    let named: Vec<&Skill> = found
        .iter()
        .filter(|s| s.id == key || slug_of(&s.text) == key)
        .collect();
    let picked: Vec<&Skill> = if named.is_empty() {
        found
            .iter()
            .filter(|s| !key.is_empty() && s.id.starts_with(key))
            .collect()
    } else {
        named
    };
    match picked.as_slice() {
        [] => Err(fail(&format!("no skill matches {key}"))),
        [one] => Ok((*one).clone()),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|s| format!("  {} {}", short_id(&s.id), slug_of(&s.text)))
                .collect();
            Err(fail(&format!(
                "{key} matches {} skills, name one:\n{}",
                many.len(),
                list.join("\n")
            )))
        }
    }
}

/// `pii skill show <id|slug>` prints the stored body verbatim under its metadata;
/// `pii skill explain <id|slug>` prints the body's own WHEN TO USE, CONTRACT and VERBS lines and
/// the verified pin. Both read the store only: a missing script is reported, not fatal.
fn show(args: &mut std::env::ArgsOs, explain: bool) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    let Some(key) = o.positional.first().cloned() else {
        return usage();
    };
    let store = match open(&o) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let found = match skills(&store, true) {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    let skill = match resolve(&found, &key) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let slug = slug_of(&skill.text);
    if !explain {
        println!("id {}", skill.id);
        println!("slug {slug}");
        println!("layer {}", layer_label(skill.layer));
        println!("concepts {}", skill.concepts.join(","));
        println!("last_used {}", skill.created_ms);
        println!("archived {}", skill.archived);
        println!("---");
        print!("{}", skill.text);
        return ExitCode::SUCCESS;
    }
    let root = current_root();
    println!("skill {} {slug}", short_id(&skill.id));
    println!(
        "layer {} · last_used {}",
        layer_label(skill.layer),
        skill.created_ms
    );
    for head in SECTIONS {
        let lines = section_lines(&skill.text, head);
        if lines.is_empty() {
            println!("{head} (absent from body)");
        }
        for line in lines {
            println!("{line}");
        }
    }
    let pins = pins_of(&skill.text);
    if pins.is_empty() {
        println!("PIN none");
    }
    for pin in &pins {
        match verify(&root, pin) {
            PinState::Ok => println!("PIN ok {} {}", pin.sha256, pin.path),
            PinState::Drift => println!(
                "PIN DRIFT pinned {} {} (the file on disk no longer matches)",
                pin.sha256, pin.path
            ),
            PinState::Missing => println!("PIN MISSING FILE {} {}", pin.sha256, pin.path),
        }
    }
    ExitCode::SUCCESS
}

/// The body's own lines from `head` up to the next section header or pin, trailing blanks dropped.
fn section_lines<'a>(text: &'a str, head: &str) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with(head) {
            inside = true;
        } else if SECTIONS.iter().any(|h| t.starts_with(h)) || t.starts_with("script:") {
            inside = false;
        }
        if inside {
            out.push(line);
        }
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out
}

/// `pii skill edit <id> --file <path> [--note <text>]`: replaces the body from a file, keeps the
/// id, records `evidence = (file:<path>, ms)`. Refuses when the new body drops a pinned script.
fn edit(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    let (Some(id), Some(file)) = (o.positional.first().cloned(), o.file.clone()) else {
        return usage();
    };
    let store = match open(&o) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let Ok(concepts) = concepts_by_doc(&store) else {
        return fail("cannot read the concept index");
    };
    let Some(old) = skill_of(&store, &concepts, &id, false) else {
        return fail(&format!("no live skill with id {id}"));
    };
    let new_text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) => return fail(&format!("read {}: {e}", file.display())),
    };
    let old_pins = pins_of(&old.text);
    let new_pins = pins_of(&new_text);
    let dropped: Vec<&str> = old_pins
        .iter()
        .filter(|p| !new_pins.iter().any(|n| n.path == p.path))
        .map(|p| p.path.as_str())
        .collect();
    if !dropped.is_empty() {
        return fail(&format!(
            "refused: the edit would drop the pin for {}; de-pinning needs a NEW skill id; body unchanged",
            dropped.join(", ")
        ));
    }

    let now_ms = pii_core::SystemClock.now_ms();
    let mut doc = Doc::new(
        layer_of_byte(old.layer),
        DocKind::Skill,
        new_text.clone(),
        old.concepts.clone(),
        1.0,
        old.created_ms,
    );
    doc.id.clone_from(&old.id);
    let mut rows = rows_for(&doc);
    // The concept edges are already in the graph; re-putting them would add their weight again.
    rows.edges.clear();
    if let Err(e) = store.put(&rows) {
        return fail(&e.to_string());
    }
    if let Err(e) = store.set_evidence(&id, &format!("file:{}", file.display()), now_ms) {
        return fail(&e.to_string());
    }

    let root = current_root();
    println!(
        "edited {}: body {} -> {} bytes",
        short_id(&id),
        old.text.len(),
        new_text.len()
    );
    println!(
        "pin before: {}  after: {}",
        pin_summary(&root, &old_pins),
        pin_summary(&root, &new_pins)
    );
    if let Some(note) = &o.note {
        println!("note: {note}");
    }
    ExitCode::SUCCESS
}

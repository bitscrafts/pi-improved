//! `pii skill {list|show|explain|add|edit}` (spec 210, spec 233): the audit surface for `DocKind::Skill` documents.
//! `list`, `show`, `explain` and `add` go through the running daemon when it serves the store they name (spec 233 R2),
//! and read or write `<dir>/memory.redb` directly when none does. A skill's pinned script is hashed where it lies
//! (relative to the current directory) and never executed.

use crate::cli_util::fail;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_core::Clock as _;
use pii_memory::doc::{Layer, layer_byte, layer_of_byte};
use pii_memory::skill::{
    AddError, Added, Edited, Listed, Pin, listing, pick, pins_of, sha256_hex, slug_of,
};
use pii_proto::method::Method;
use pii_store::MemStore;

use crate::cmd_ctl::{CallError, next_str, run_call_typed};

const USAGE: &str = "usage: pii skill add --file <path> [--layer L1|L2|L3] [--dir D]\n       \
pii skill list [--all] [--layer L1|L2|L3] [--dir D]\n       \
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

/// `pii skill {list|show|explain|add|edit} ...`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_skill(args: &mut std::env::ArgsOs) -> ExitCode {
    match next_str(args).as_deref() {
        Some("add") => add(args),
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

/// Names the source that answered a read or a write (spec 233 R2): `via daemon`, or `via store <path>`.
fn note_source(via: &str) {
    eprintln!("via {via}");
}

/// The memory store file a command names: `--dir`, else the runtime dir, which is where the daemon keeps its store.
/// It must already exist, so a mistyped `--dir` cannot quietly create an empty memory file.
fn store_file(o: &Opts) -> Result<PathBuf, ExitCode> {
    let dir = o.dir.clone().unwrap_or_else(pii_core::state_dir);
    let path = dir.join("memory.redb");
    if !path.exists() {
        return Err(fail(
            "skill",
            format!("no memory store at {}", path.display()),
        ));
    }
    Ok(path)
}

fn open_file(path: &Path) -> Result<MemStore, ExitCode> {
    MemStore::open(path).map_err(|e| fail("skill", e.to_string()))
}

/// Whether the daemon serves the store this command names (spec 233 R2). A command with no `--dir`, or the runtime
/// dir itself, is the daemon's store. Any other `--dir` is a different store, and the daemon would answer for its own.
fn daemon_serves(o: &Opts) -> bool {
    o.dir.is_none() || o.dir == Some(pii_core::state_dir())
}

/// Asks the running daemon through `method`. `None` when no daemon is reachable, so the caller reads the store.
/// `Some(Err)` when the daemon answered with an error: that is the answer, and the store is never opened in its place,
/// because the daemon holds it.
fn ask_daemon(
    o: &Opts,
    method: Method,
    params: serde_json::Value,
) -> Option<Result<serde_json::Value, ExitCode>> {
    if !daemon_serves(o) {
        return None;
    }
    match run_call_typed(method, params) {
        Ok(reply) => Some(Ok(reply)),
        Err(CallError::Down(_)) => None,
        Err(CallError::Failed(msg)) => Some(Err(fail("skill", &msg))),
    }
}

/// The skill documents the store holds, archived ones only when `with_archived` is set, with the source that answered
/// them printed (spec 233 R2). The daemon answers when it serves the store; otherwise the store file is read directly.
fn read_skills(o: &Opts, with_archived: bool) -> Result<Vec<Listed>, ExitCode> {
    let (found, via) = fetch_skills(o, with_archived)?;
    note_source(&via);
    Ok(found)
}

/// The skill documents [`read_skills`] reads, and the source that answered them (`daemon`, or `store <path>`). Prints
/// nothing, so a caller that reads and then writes can name the source once.
fn fetch_skills(o: &Opts, with_archived: bool) -> Result<(Vec<Listed>, String), ExitCode> {
    let params = serde_json::json!({ "all": with_archived });
    if let Some(outcome) = ask_daemon(o, Method::SkillList, params) {
        let reply = outcome?;
        let found = serde_json::from_value(reply).map_err(|e| {
            fail(
                "skill",
                format!("the daemon's skill list does not decode: {e}"),
            )
        })?;
        return Ok((found, String::from("daemon")));
    }
    let path = store_file(o)?;
    let store = open_file(&path)?;
    let found = listing(&store, with_archived).map_err(|e| fail("skill", e.to_string()))?;
    Ok((found, format!("store {}", path.display())))
}

fn current_root() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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

fn short_id(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

/// `pii skill add --file <path> [--layer L] [--dir D]` (spec 233 R1): stores the file's text as one skill document,
/// through the daemon when it serves the store, else into the store. Prints `added <id> <slug> sha256=<hex>`.
fn add(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    let Some(file) = o.file.clone() else {
        return usage();
    };
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) => return fail("skill", format!("read {}: {e}", file.display())),
    };
    let layer = o.layer.unwrap_or_else(|| layer_byte(Layer::L3));
    let params = serde_json::json!({ "text": text, "layer": layer });
    let added = ask_daemon(&o, Method::SkillPut, params).map_or_else(
        || add_to_store(&o, &text, layer),
        |outcome| {
            outcome.and_then(|reply| {
                note_source("daemon");
                serde_json::from_value::<Added>(reply).map_err(|e| {
                    fail(
                        "skill",
                        format!("the daemon's add reply does not decode: {e}"),
                    )
                })
            })
        },
    );
    match added {
        Ok(a) => {
            println!("added {} {} sha256={}", a.id, a.slug, a.sha256);
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// The store-direct half of `add`, for when no daemon serves the store.
fn add_to_store(o: &Opts, text: &str, layer: u8) -> Result<Added, ExitCode> {
    let path = store_file(o)?;
    let store = open_file(&path)?;
    note_source(&format!("store {}", path.display()));
    let now_ms = pii_core::SystemClock.now_ms();
    pii_memory::skill::add(&store, text, layer_of_byte(layer), now_ms).map_err(|e| match e {
        AddError::NoMarker => fail(
            "skill",
            "the file has no PII-SKILL marker line, so it is not a skill",
        ),
        AddError::Taken { slug, id } => fail(
            "skill",
            format!("skill {slug} is already live as {id}; change it with pii skill edit"),
        ),
        AddError::Store(e) => fail("skill", e.to_string()),
    })
}

/// `pii skill list [--all] [--layer L] [--dir D]`: one line per skill.
fn list(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    let found = match read_skills(&o, o.all) {
        Ok(f) => f,
        Err(code) => return code,
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
fn resolve(found: &[Listed], key: &str) -> Result<Listed, ExitCode> {
    let picked = pick(found, key, |s| (s.id.as_str(), s.text.as_str()));
    match picked.as_slice() {
        [] => Err(fail("skill", format!("no skill matches {key}"))),
        [one] => Ok((*one).clone()),
        many => {
            let list: Vec<String> = many
                .iter()
                .map(|s| format!("  {} {}", short_id(&s.id), slug_of(&s.text)))
                .collect();
            Err(fail(
                "skill",
                format!(
                    "{key} matches {} skills, name one:\n{}",
                    many.len(),
                    list.join("\n")
                ),
            ))
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
    let found = match read_skills(&o, true) {
        Ok(f) => f,
        Err(code) => return code,
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

/// `pii skill edit <id> --file <path> [--note <text>]`: replaces the body from a file, keeps the id, records
/// `evidence = (file:<path>, ms)`. Through the daemon when it serves the store (spec 234 R3), else into the store.
/// Refuses when the new body drops a pinned script, and when it has no marker.
fn edit(args: &mut std::env::ArgsOs) -> ExitCode {
    let Some(o) = parse(args) else {
        return usage();
    };
    let (Some(id), Some(file)) = (o.positional.first().cloned(), o.file.clone()) else {
        return usage();
    };
    let new_text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) => return fail("skill", format!("read {}: {e}", file.display())),
    };
    // The body as it stands, for the pin status the command reports before the edit.
    let (found, _) = match fetch_skills(&o, false) {
        Ok(f) => f,
        Err(code) => return code,
    };
    let old_pins = found
        .iter()
        .find(|s| s.id == id)
        .map(|s| pins_of(&s.text))
        .unwrap_or_default();
    let source = format!("file:{}", file.display());
    let params = serde_json::json!({ "id": id, "text": new_text, "source": source });
    let edited = ask_daemon(&o, Method::SkillEdit, params).map_or_else(
        || edit_in_store(&o, &id, &new_text, &source),
        |outcome| {
            outcome.and_then(|reply| {
                note_source("daemon");
                serde_json::from_value::<Edited>(reply).map_err(|e| {
                    fail(
                        "skill",
                        format!("the daemon's edit reply does not decode: {e}"),
                    )
                })
            })
        },
    );
    let edited = match edited {
        Ok(e) => e,
        Err(code) => return code,
    };

    let root = current_root();
    println!(
        "edited {}: body {} -> {} bytes",
        short_id(&id),
        edited.bytes_before,
        edited.bytes_after
    );
    println!(
        "pin before: {}  after: {}",
        pin_summary(&root, &old_pins),
        pin_summary(&root, &pins_of(&new_text))
    );
    if let Some(note) = &o.note {
        println!("note: {note}");
    }
    ExitCode::SUCCESS
}

/// The store-direct half of `edit`, for when no daemon serves the store.
fn edit_in_store(o: &Opts, id: &str, text: &str, source: &str) -> Result<Edited, ExitCode> {
    let path = store_file(o)?;
    let store = open_file(&path)?;
    note_source(&format!("store {}", path.display()));
    let now_ms = pii_core::SystemClock.now_ms();
    pii_memory::skill::edit(&store, id, text, now_ms, source)
        .map_err(|e| fail("skill", e.to_string()))
}

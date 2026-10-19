//! `pii index code [--root D] [--max N] [--dir D] [--no-docs]` (spec 204): pii indexes its own
//! Rust workspace into its memory graph. It walks `<root>/crates/*/src`, writes entities and the
//! two relationship families (`contains`, `depends_on`) through [`EntityStore`], and — when a
//! daemon answers — one memory document per module whose concepts are its symbols, so the
//! search graph lane can fire for a query naming one. Every edge comes from a parsed `mod`
//! declaration, a symbol's lexical containment, or a Cargo.toml dependency. Re-running is
//! idempotent: records are keyed on kind+name and provenance is refreshed, never multiplied.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pii_memory::entity::{EntityInput, EntityStore, GraphConfig, MAX_NAME_BYTES, MAX_SOURCE_BYTES};
use pii_proto::method::Method;

use crate::cmd_ctl::{next_str, run_call_typed};

const USAGE: &str = "usage: pii index code [--root <dir>] [--max N] [--dir D] [--no-docs]";
const SKIP_DIRS: [&str; 3] = ["target", "_tmp", "vendor"];
const DOC_CONCEPTS: usize = 64;
const DOC_TEXT_BYTES: usize = 1_500;

struct Symbol {
    kind: &'static str,
    name: String,
    line: usize,
}

struct Module {
    name: String,
    file: String,
    lines: usize,
    declared: Vec<String>,
    symbols: Vec<Symbol>,
}

fn fail(msg: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "error: {msg}");
    ExitCode::from(2)
}

/// Cuts `s` to at most `max` bytes on a char boundary.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn clip(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

/// `pii index code ...`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_index(args: &mut std::env::ArgsOs) -> ExitCode {
    let verb = next_str(args);
    if verb.as_deref() == Some("docs") {
        return crate::cmd_index_docs::cmd_docs(args);
    }
    if verb.as_deref() != Some("code") {
        let _ = writeln!(std::io::stderr(), "{USAGE}");
        return ExitCode::from(2);
    }
    let mut root = PathBuf::from(".");
    let mut dir = pii_uds::runtime_dir("default");
    let mut max = 3000_usize;
    let mut docs = true;
    while let Some(a) = next_str(args) {
        let ok = match a.as_str() {
            "--root" => next_str(args).map(|v| root = PathBuf::from(v)).is_some(),
            "--dir" => next_str(args)
                .map(|v| dir = Some(PathBuf::from(v)))
                .is_some(),
            "--max" => next_str(args)
                .and_then(|v| v.parse().ok())
                .map(|v| max = v)
                .is_some(),
            "--no-docs" => {
                docs = false;
                true
            }
            _ => false,
        };
        if !ok {
            let _ = writeln!(std::io::stderr(), "{USAGE}");
            return ExitCode::from(2);
        }
    }
    let Some(dir) = dir else {
        return fail("no store directory (use --dir)");
    };
    let crates = match scan_crates(&root) {
        Ok(c) if !c.is_empty() => c,
        Ok(_) => return fail(&format!("no crates/*/src under {}", root.display())),
        Err(e) => return fail(&e),
    };
    let cfg = GraphConfig {
        max_entities: GraphConfig::default()
            .max_entities
            .max(max.saturating_mul(2)),
        ..GraphConfig::default()
    };
    let store = match EntityStore::open(&dir, cfg) {
        Ok(s) => s,
        Err(e) => return fail(&e.to_string()),
    };
    let mut run = Run {
        store,
        max,
        entities: BTreeSet::new(),
        relationships: BTreeSet::new(),
        failed: 0,
        docs: 0,
        docs_live: docs,
    };
    run.index(&crates);
    let _ = writeln!(
        std::io::stdout(),
        "entities={} relationships={} failed={} docs={} crates={}",
        run.entities.len(),
        run.relationships.len(),
        run.failed,
        run.docs,
        crates.len()
    );
    ExitCode::SUCCESS
}

struct CrateInfo {
    name: String,
    deps: Vec<String>,
    modules: Vec<Module>,
}

fn skipped(p: &Path) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| SKIP_DIRS.contains(&n))
}

fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let rd = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut v: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    v.sort();
    Ok(v)
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for p in sorted_entries(dir)? {
        if skipped(&p) {
            continue;
        }
        if p.is_dir() {
            rs_files(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    Ok(())
}

fn scan_crates(root: &Path) -> Result<Vec<CrateInfo>, String> {
    let base = root.join("crates");
    if !base.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for cdir in sorted_entries(&base)? {
        let src = cdir.join("src");
        let Some(name) = cdir.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !src.is_dir() || skipped(&cdir) {
            continue;
        }
        let mut files = Vec::new();
        rs_files(&src, &mut files)?;
        let modules = files
            .iter()
            .filter_map(|f| parse_module(root, &src, name, f))
            .collect();
        let manifest = std::fs::read_to_string(cdir.join("Cargo.toml")).unwrap_or_default();
        out.push(CrateInfo {
            name: name.to_owned(),
            deps: manifest_deps(&manifest),
            modules,
        });
    }
    Ok(out)
}

/// Names under `[dependencies]` of a Cargo.toml (the caller keeps those that are workspace crates).
fn manifest_deps(manifest: &str) -> Vec<String> {
    let mut in_deps = false;
    let mut out = Vec::new();
    for line in manifest.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_deps = l == "[dependencies]";
        } else if in_deps && let Some((k, _)) = l.split_once('=') {
            let k = k.trim();
            if !k.is_empty() && !k.starts_with('#') {
                out.push(k.to_owned());
            }
        }
    }
    out
}

fn module_name(src: &Path, krate: &str, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(src).ok()?.with_extension("");
    let mut parts: Vec<String> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_owned))
        .collect();
    if matches!(
        parts.last().map(String::as_str),
        Some("mod" | "lib" | "main")
    ) {
        parts.pop();
    }
    parts.insert(0, krate.to_owned());
    Some(parts.join("::"))
}

fn parse_module(root: &Path, src: &Path, krate: &str, file: &Path) -> Option<Module> {
    let text = std::fs::read_to_string(file).ok()?;
    let rel = file
        .strip_prefix(root)
        .unwrap_or(file)
        .display()
        .to_string();
    let name = module_name(src, krate, file)?;
    let mut m = Module {
        file: rel,
        lines: text.lines().count(),
        declared: Vec::new(),
        symbols: Vec::new(),
        name,
    };
    for (i, raw) in text.lines().enumerate() {
        let toplevel = !raw.starts_with(char::is_whitespace);
        let mut rest = raw.trim_start();
        if let Some(r) = rest.strip_prefix("pub") {
            rest = match r.strip_prefix('(').and_then(|x| x.split_once(')')) {
                Some((_, after)) => after.trim_start(),
                None => r.trim_start(),
            };
        }
        while let Some(r) = ["async ", "const ", "unsafe "]
            .iter()
            .find_map(|q| rest.strip_prefix(q))
        {
            rest = r;
        }
        let ident = |s: &str| -> String {
            s.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        };
        for (kw, kind) in [
            ("fn ", "fn"),
            ("struct ", "struct"),
            ("trait ", "trait"),
            ("enum ", "enum"),
        ] {
            if let Some(r) = rest.strip_prefix(kw) {
                let n = ident(r);
                if !n.is_empty() {
                    m.symbols.push(Symbol {
                        kind,
                        name: n,
                        line: i + 1,
                    });
                }
            }
        }
        if toplevel && let Some(r) = rest.strip_prefix("mod ") {
            let n = ident(r);
            if !n.is_empty()
                && r.strip_prefix(n.as_str())
                    .is_some_and(|t| t.trim_start().starts_with(';'))
            {
                m.declared.push(n);
            }
        }
    }
    Some(m)
}

struct Run {
    store: EntityStore,
    max: usize,
    entities: BTreeSet<String>,
    relationships: BTreeSet<String>,
    failed: usize,
    docs: usize,
    docs_live: bool,
}

impl Run {
    fn full(&self) -> bool {
        self.entities.len() >= self.max
    }

    fn entity(
        &mut self,
        kind: &str,
        name: &str,
        source: &str,
        attrs: &[(&str, String)],
    ) -> Option<String> {
        let mut input = EntityInput::new(kind, clip(name, MAX_NAME_BYTES))
            .source(clip(source, MAX_SOURCE_BYTES));
        for (k, v) in attrs {
            input = input.attr(*k, v.clone());
        }
        if let Ok(p) = self.store.put_entity(input) {
            self.entities.insert(p.record.id.clone());
            Some(p.record.id)
        } else {
            self.failed += 1;
            None
        }
    }

    fn link(&mut self, from: &str, to: &str, rel: &str, source: &str) {
        let input = pii_memory::entity::RelationshipInput::new(from, to, rel)
            .source(clip(source, MAX_SOURCE_BYTES));
        match self.store.put_relationship(input) {
            Ok(p) => {
                self.relationships.insert(p.record.id);
            }
            Err(_) => self.failed += 1,
        }
    }

    fn index(&mut self, crates: &[CrateInfo]) {
        let mut crate_ids: BTreeMap<&str, String> = BTreeMap::new();
        for c in crates {
            let src = format!("file:crates/{}/Cargo.toml", c.name);
            if let Some(id) = self.entity("crate", &c.name, &src, &[]) {
                crate_ids.insert(c.name.as_str(), id);
            }
        }
        for c in crates {
            let Some(from) = crate_ids.get(c.name.as_str()).cloned() else {
                continue;
            };
            for d in &c.deps {
                if let Some(to) = crate_ids.get(d.as_str()).cloned() {
                    let src = format!("file:crates/{}/Cargo.toml", c.name);
                    self.link(&from, &to, "depends_on", &src);
                }
            }
        }
        let known: BTreeSet<&str> = crates
            .iter()
            .flat_map(|c| c.modules.iter().map(|m| m.name.as_str()))
            .collect();
        for c in crates {
            for m in &c.modules {
                if self.full() {
                    return;
                }
                self.module(c, m, &known);
            }
        }
    }

    fn module(&mut self, c: &CrateInfo, m: &Module, known: &BTreeSet<&str>) {
        let src = format!("file:{}:1", m.file);
        let attrs = [("crate", c.name.clone()), ("lines", m.lines.to_string())];
        let Some(mid) = self.entity("module", &m.name, &src, &attrs) else {
            return;
        };
        for d in &m.declared {
            let child = format!("{}::{d}", m.name);
            if known.contains(child.as_str())
                && let Some(cid) = self.entity("module", &child, &src, &[])
            {
                self.link(&mid, &cid, "contains", &src);
            }
        }
        for s in &m.symbols {
            if self.full() {
                break;
            }
            let src = format!("file:{}:{}", m.file, s.line);
            let name = format!("{}::{}", m.name, s.name);
            if let Some(sid) = self.entity(s.kind, &name, &src, &[("crate", c.name.clone())]) {
                self.link(&mid, &sid, "contains", &src);
            }
        }
        if self.docs_live {
            self.doc(m);
        }
    }

    /// One memory document per module; its concepts are the module path and symbol tokens.
    fn doc(&mut self, m: &Module) {
        if m.symbols.is_empty() {
            return;
        }
        let names: Vec<&str> = m.symbols.iter().map(|s| s.name.as_str()).collect();
        let text = clip(
            &format!("module {} contains {}", m.name, names.join(", ")),
            DOC_TEXT_BYTES,
        );
        let mut concepts: Vec<String> =
            pii_memory::tokenize(&format!("{} {}", m.name, names.join(" ")));
        let mut seen = BTreeSet::new();
        concepts.retain(|c| seen.insert(c.clone()));
        concepts.truncate(DOC_CONCEPTS);
        match run_call_typed(
            Method::MemoryPut,
            serde_json::json!({"text": text, "concepts": concepts}),
        ) {
            Ok(_) => self.docs += 1,
            Err(_) => self.docs_live = false,
        }
    }
}

//! The scratch fixture and the readers shared by the catalogue tests (spec 388).
//!
//! The fixture runs the built `pii` binary and reads its stdout and stderr; the lints allowed below are for that,
//! not for the tests themselves.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::disallowed_methods,
    clippy::new_without_default,
    clippy::missing_panics_doc,
    clippy::print_stderr // a failing test's only surviving evidence is its own captured stderr
)]

use pii_store::MemStore;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A scratch root: `graphs/` is a copy of the shipped directory, and the state dir is the user's own.
pub struct Fixture {
    /// The scratch root, under the crate's `target/` scratch pool.
    pub root: tempfile::TempDir,
}

impl Fixture {
    /// Creates the root: `graphs/` a copy of the shipped directory, and the state dir the user's own.
    pub fn new() -> Self {
        // The state/socket dir sits four levels under the scratch root and a Unix socket path must stay under
        // SUN_LEN (108): the gate runs inside `/projects/_tmp/fresh-gate-<id>/`, 12 chars longer than the main
        // tree, so the scratch prefix must stay three characters ('gx-'), and the root stays in the crate's own
        // `_tmp` (the sanctioned pool lint-scratch requires).
        let root = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "gx-").unwrap();
        let this = Self { root };
        std::fs::create_dir_all(this.state()).unwrap();
        copy_graphs(&shipped_graphs(), &this.graphs());
        this
    }

    /// The graph directory inside the root.
    pub fn graphs(&self) -> PathBuf {
        self.root.path().join("graphs")
    }

    /// The `HOME` the command runs with.
    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    /// The default state dir under `HOME`, so `<state>/graphs` is the user graph directory by every rule that names it.
    pub fn state(&self) -> PathBuf {
        self.home().join(".local/state/pii")
    }

    /// Runs `cmd` in the root, with `HOME` and both state dirs inside it.
    pub fn envs<'a>(&self, cmd: &'a mut Command) -> &'a mut Command {
        cmd.current_dir(self.root.path())
            .env("HOME", self.home())
            .env("PII_RUNTIME_DIR", self.state())
            // State (and its durable stores) must be this test's own: the operator's are
            // held by any daemon already running on the machine.
            .env("PII_STATE_DIR", self.state())
            .env("PII_CONNECT_TIMEOUT_MS", "300")
            .stdin(Stdio::null())
    }

    /// Runs the built binary with `args` inside the fixture and returns its output.
    pub fn pii(&self, args: &[&str]) -> Output {
        self.envs(&mut Command::new(env!("CARGO_BIN_EXE_pii")))
            .args(args)
            .output()
            .unwrap()
    }

    /// Runs the index over the copied directory and returns the count it reports. The path is relative, as the exit
    /// criterion spells it.
    pub fn index(&self) -> usize {
        let out = self.pii(&["graph", "index", "--dir", "graphs"]);
        let text = combined(&out);
        indexed_count(&text).unwrap_or_else(|| panic!("no 'indexed N graphs' line: {text}"))
    }

    /// Runs `pii graph find` in the fixture.
    pub fn find(&self, question: &str) -> Output {
        self.pii(&["graph", "find", question])
    }

    /// Opens the fixture's own `memory.redb` directly (tests that answer through the daemon must not).
    pub fn memory(&self) -> MemStore {
        MemStore::open(&self.state().join("memory.redb")).unwrap()
    }
}

/// The workspace's shipped `graphs/` directory.
pub fn shipped_graphs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("graphs")
}

/// Copies the `.toml` files under `from`, keeping their relative paths.
pub fn copy_graphs(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let target = to.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_graphs(&path, &target);
        } else if path.extension().is_some_and(|e| e == "toml") {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// Every `.toml` file under `dir`, deepest paths included.
pub fn toml_files_under(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            toml_files_under(&path, files);
        } else if path.extension().is_some_and(|x| x == "toml") {
            files.push(path);
        }
    }
}

/// One shipped file, parsed the way the catalogue is meant to read it.
/// One shipped file, parsed the way the catalogue is meant to read it.
pub struct Shipped {
    /// The graph's file name without `.toml`.
    pub name: String,
    /// True for the one shipped file that is a fragment, not a graph on its own.
    pub is_fragment: bool,
    /// The node kinds the graph holds, parsed.
    pub kinds: BTreeSet<String>,
    /// The graphs a `call` node targets.
    pub calls: BTreeSet<String>,
}

/// Parses every shipped graph, so a test can expect the kinds and calls its question names.
pub fn shipped() -> Vec<Shipped> {
    let mut files = Vec::new();
    toml_files_under(&shipped_graphs(), &mut files);
    assert!(!files.is_empty(), "no graphs found under graphs/");
    files
        .iter()
        .map(|file| {
            let text = std::fs::read_to_string(file).unwrap();
            let doc: toml::Table = toml::from_str(&text)
                .unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
            let nodes = doc
                .get("node")
                .and_then(toml::Value::as_array)
                .cloned()
                .unwrap_or_default();
            let field = |node: &toml::Value, key: &str| {
                node.get(key)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
            };
            Shipped {
                name: doc
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .unwrap_or_else(|| panic!("{} has no name", file.display()))
                    .to_owned(),
                is_fragment: doc.contains_key("fragment"),
                kinds: nodes.iter().filter_map(|n| field(n, "kind")).collect(),
                calls: nodes
                    .iter()
                    .filter(|n| field(n, "kind").as_deref() == Some("call"))
                    .filter_map(|n| field(n, "graph"))
                    .collect(),
            }
        })
        .collect()
}

/// The one shipped graph named `name`, found by parse.
pub fn shipped_named(name: &str) -> Shipped {
    shipped()
        .into_iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("no shipped graph named {name}"))
}

/// `stdout` joined with `stderr`, so a failure message and an answer both reach the reader.
pub fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The `N` of the first `indexed N graphs` in `text`.
pub fn indexed_count(text: &str) -> Option<usize> {
    text.lines().find_map(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        words
            .windows(3)
            .find(|w| w[0] == "indexed" && w[2] == "graphs")
            .and_then(|w| w[1].parse().ok())
    })
}

/// The first word of `line`, without a list marker or trailing punctuation: `1. books:` reads `books`.
pub fn first_word(line: &str) -> Option<&str> {
    line.split_whitespace()
        .find(|w| !matches!(*w, "-" | "*" | "#") && w.trim_end_matches('.').parse::<u32>().is_err())
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
}

/// One candidate in an answer: its name, and the lines that belong to it.
pub struct Hit {
    /// The graph's name, as the answer shows it.
    pub name: String,
    /// The answer's lines from the name line down.
    pub lines: Vec<String>,
}

/// Reads the answer into hits, and the unindented lines that belong to none (see the module doc).
pub fn read_answer(stdout: &str) -> (Vec<Hit>, Vec<String>) {
    let names: BTreeSet<String> = shipped().into_iter().map(|g| g.name).collect();
    let mut hits: Vec<Hit> = Vec::new();
    let mut outside = Vec::new();
    let mut in_hit = false;
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        if line.starts_with(char::is_whitespace) {
            if in_hit {
                hits.last_mut().unwrap().lines.push(line.to_owned());
            } else {
                outside.push(line.to_owned());
            }
        } else if let Some(name) = first_word(line).filter(|w| names.contains(*w)) {
            hits.push(Hit {
                name: name.to_owned(),
                lines: vec![line.to_owned()],
            });
            in_hit = true;
        } else {
            outside.push(line.to_owned());
            in_hit = false;
        }
    }
    (hits, outside)
}

/// Whether `word` stands alone in `line`, so `docs` is not matched by `docs-writer`.
pub fn names_word(line: &str, word: &str) -> bool {
    line.split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '/'))
        .any(|w| w == word)
}

/// Sums every count the store keeps about the catalogue: documents, indexed length, concepts and edges.
pub fn store_counts(store: &MemStore) -> (u64, u64, usize, usize) {
    let concepts = store.concepts().unwrap();
    let edges = concepts.iter().map(|c| store.edges(c).unwrap().len()).sum();
    (
        store.n_docs().unwrap(),
        store.sum_dl().unwrap(),
        concepts.len(),
        edges,
    )
}

/// Notes a failing run's evidence on stderr: under the gate the test's captured output is the only thing that
/// survives (the scratch tree is deleted with the test's own drop).
#[allow(clippy::print_stderr)] // reason: the evidence channel, not a print seam
pub fn note(msg: &str) {
    eprintln!("{msg}");
}

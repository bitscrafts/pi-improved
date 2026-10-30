//! Spec 317 acceptance: `pii lane plan` against the composed lane's own output. Goldens live in
//! `tests/golden/lane` (captured from `scripts/run-batchC-pii.sh`, see CAPTURE.md). Every test runs in its own
//! scratch clone, so nothing touches the live `_tmp/claude-batchC`.
#![allow(
    clippy::disallowed_methods, // reason: the tests drive git and the shell reference directly
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    clippy::format_push_string,
    clippy::items_after_statements,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::format_collect,
    clippy::map_unwrap_or,
    clippy::option_if_let_else,
    clippy::semicolon_if_nothing_returned
)]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_pii");
/// The goldens' byte counts (`(N bytes)`) were captured in a clone whose root is 51 bytes long
/// (the golden brief is 1360 bytes on disk with `<REPO>` for 51 bytes: 1405 printed). The scratch root matches it.
const REPO_LEN: usize = 51;
/// The lane's variables, cleared before each run so the test host's environment cannot leak in.
const LANE_VARS: [&str; 11] = [
    "ALLOW_DIRTY",
    "PII_AGENT",
    "PII_LANE_GRAPH",
    "PII_GRAPH_EDIT_OK",
    "PII_MAX_BUDGET_USD",
    "PII_NODE_TIMEOUT_MS",
    "PII_WAIT_MARGIN_S",
    "CLAUDE_MODEL",
    "PI_MODEL",
    "PI_PROVIDER",
    "DEVBOX_HOST",
];
/// Declared exclusions from the stderr comparison (spec 317 "named exclusion list"):
/// the shell's Python traceback for an unparseable budget, and bash's `<script>: line N:` prefix on `--resume`.
const STDERR_EXCLUDED: [&str; 1] = ["spec-budget-bad"];
const BASH_RESUME_PREFIX: &str = "<SCRATCH>/lane.sh: line 39: 2: ";
static NEXT: AtomicUsize = AtomicUsize::new(0);

fn live() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden_dir(case: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/lane")
        .join(case)
}

/// Scratch setup options: paths to leave out of the commit (untracked on disk) or to delete entirely.
#[derive(Default)]
struct Opts {
    ignore: &'static str,
    remove: &'static [&'static str],
}

/// A scratch clone of the live specs, graphs and script, committed; plus an outside-the-repo directory for extra briefs.
struct Scratch {
    root: PathBuf,
    extra: PathBuf,
}

/// The scratch base: a directory outside every `_tmp` component (the port refuses a root inside `_tmp/`, R1, and
/// both the gate's target dir and a fresh-gate export live under one), short enough to pad a unique name up to
/// [`REPO_LEN`], and disposable. The runtime dir comes first - tmpfs, and the sanctioned root for scratch that
/// cannot live in the tree; `CARGO_TARGET_TMPDIR` keeps an ordinary `cargo test` inside `target/`; `/var/tmp` is
/// the last resort. Never `/tmp`: the project's rule is that no test roots scratch there.
fn scratch_base() -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        candidates.push(PathBuf::from(rt));
    }
    candidates.push(PathBuf::from(env!("CARGO_TARGET_TMPDIR")));
    candidates.push(PathBuf::from("/var/tmp"));
    for candidate in candidates {
        let Ok(candidate) = fs::canonicalize(candidate) else {
            continue;
        };
        let base = candidate.join("pii-lane317");
        let outside_tmp = !base.components().any(|c| c.as_os_str() == "_tmp");
        let fits = base.as_os_str().len() + 8 < REPO_LEN;
        if outside_tmp && fits && fs::create_dir_all(&base).is_ok() {
            return base;
        }
    }
    panic!("no scratch base free of _tmp and short enough for the {REPO_LEN} byte golden root");
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A failing test keeps its clone: the clone is the evidence. A passing one removes it, so a gate run does
        // not leave forty clones behind in the runtime dir. PII_KEEP_SCRATCH=1 keeps them either way.
        if std::thread::panicking() || std::env::var_os("PII_KEEP_SCRATCH").is_some() {
            return;
        }
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(&self.extra);
    }
}

impl Scratch {
    fn new(opts: &Opts) -> Self {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let base = scratch_base();
        let width = REPO_LEN - base.as_os_str().len() - 1;
        let name = format!("{:0>width$}", format!("{:x}{n:x}", std::process::id()));
        assert!(
            name.len() == width,
            "scratch name does not fit the root length"
        );
        let root = base.join(name);
        let extra = base.join(format!("extra-{}-{n}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&extra).unwrap();
        let status = Command::new("cp")
            .arg("-r")
            .arg(live().join("specs"))
            .arg(live().join("graphs"))
            .arg(live().join("scripts"))
            .arg(&root)
            .status()
            .unwrap();
        assert!(status.success());
        for r in opts.remove {
            fs::remove_file(root.join(r)).unwrap();
        }
        let mut ignore = String::from("_tmp/\n");
        if !opts.ignore.is_empty() {
            ignore.push_str(opts.ignore);
            ignore.push('\n');
        }
        fs::write(root.join(".gitignore"), ignore).unwrap();
        let s = Self { root, extra };
        s.git(&["init", "-q", "."]);
        s.git(&["add", "-A"]);
        s.git(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "base",
        ]);
        s
    }

    fn git(&self, args: &[&str]) -> Output {
        let out = Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    fn head(&self) -> String {
        String::from_utf8_lossy(&self.git(&["rev-parse", "HEAD"]).stdout)
            .trim_end()
            .to_owned()
    }

    fn append(&self, rel: &str, text: &str) {
        let p = self.root.join(rel);
        let mut body = fs::read_to_string(&p).unwrap();
        body.push_str(text);
        fs::write(p, body).unwrap();
    }

    fn replace(&self, rel: &str, from: &str, to: &str) {
        let p = self.root.join(rel);
        let body = fs::read_to_string(&p).unwrap().replace(from, to);
        fs::write(p, body).unwrap();
    }

    fn prepend(&self, rel: &str, text: &str) {
        let p = self.root.join(rel);
        let body = fs::read_to_string(&p).unwrap();
        fs::write(p, format!("{text}{body}")).unwrap();
    }

    fn pii(&self, env: &[(&str, &str)], args: &[&str]) -> Output {
        let mut c = Command::new(BIN);
        c.current_dir(&self.root).args(["lane", "plan"]);
        c.args(
            args.iter()
                .map(|a| a.replace("<SCRATCH>", &self.extra.to_string_lossy())),
        );
        for v in LANE_VARS {
            c.env_remove(v);
        }
        c.envs(env.iter().copied());
        c.output().unwrap()
    }

    /// The composed lane itself: the live script rewritten to this clone (CAPTURE.md steps 2-3).
    fn shell(&self, env: &[(&str, &str)], args: &[&str]) -> Output {
        let script = self.extra.join("run-batchC-pii.sh");
        let text = fs::read_to_string(live().join("scripts/run-batchC-pii.sh")).unwrap();
        let mut rewritten = String::new();
        for line in text.lines() {
            if line.starts_with("REPO=") {
                rewritten.push_str(&format!("REPO={}\n", self.root.display()));
            } else if line.starts_with("C=") {
                rewritten.push_str(&format!("C={}/_tmp/devbox-test\n", self.root.display()));
            } else {
                rewritten.push_str(line);
                rewritten.push('\n');
            }
        }
        fs::write(&script, rewritten).unwrap();
        let mut c = Command::new("bash");
        c.arg(&script)
            .args(
                args.iter()
                    .map(|a| a.replace("<SCRATCH>", &self.extra.to_string_lossy())),
            )
            .current_dir(&self.root)
            .env("DEVBOX_HOST", "nobody@127.0.0.1")
            // uv's default cache is under /root, which this sandbox cannot write; the cache is scratch either way.
            .env("UV_CACHE_DIR", self.extra.join("uv-cache"));
        for v in LANE_VARS {
            c.env_remove(v);
        }
        c.envs(env.iter().copied());
        c.output().unwrap()
    }

    /// The files the run wrote under the two lane directories, names and bytes normalised.
    fn files(&self, seen: &mut Vec<String>) -> Vec<(String, Vec<u8>)> {
        let head = self.head();
        let mut out = Vec::new();
        for dir in ["_tmp/claude-batchC", "_tmp/devbox-test/graphs"] {
            let Ok(rd) = fs::read_dir(self.root.join(dir)) else {
                continue;
            };
            for e in rd.flatten() {
                if !e.path().is_file() {
                    continue;
                }
                let name = norm(
                    e.file_name().to_string_lossy().as_bytes(),
                    self,
                    &head,
                    seen,
                );
                let body = norm(&fs::read(e.path()).unwrap(), self, &head, seen);
                out.push((String::from_utf8(name).unwrap(), body));
            }
        }
        out.sort();
        out
    }

    fn logs(&self) -> usize {
        fs::read_dir(self.root.join("_tmp/claude-batchC"))
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "log"))
                    .count()
            })
            .unwrap_or(0)
    }
}

/// Exact substitutions (`<REPO>`, `<HEAD>`, `<SCRATCH>`, the script path) and one ten-digit `<TS>`, collected in `seen`.
fn norm(bytes: &[u8], s: &Scratch, head: &str, seen: &mut Vec<String>) -> Vec<u8> {
    let mut text = String::from_utf8_lossy(bytes).into_owned();
    text = text.replace(&s.root.display().to_string(), "<REPO>");
    text = text.replace(
        &s.extra.join("run-batchC-pii.sh").display().to_string(),
        "<SCRIPT>",
    );
    text = text.replace(&s.extra.display().to_string(), "<SCRATCH>");
    if !head.is_empty() {
        text = text.replace(head, "<HEAD>");
    }
    let mut out = String::new();
    let mut run = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            run.push(c);
            continue;
        }
        if run.len() == 10 {
            seen.push(run.clone());
            out.push_str("<TS>");
        } else {
            out.push_str(&run);
        }
        run.clear();
        out.push(c);
    }
    if run.len() == 10 {
        seen.push(run.clone());
        out.push_str("<TS>");
    } else {
        out.push_str(&run);
    }
    out.into_bytes()
}

/// `CASE`: `env:` words and `args:` words.
fn read_case(case: &str) -> (Vec<(String, String)>, Vec<String>) {
    let text = fs::read_to_string(golden_dir(case).join("CASE")).unwrap();
    let mut env = Vec::new();
    let mut args = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("env:") {
            for kv in rest.split_whitespace() {
                let (k, v) = kv.split_once('=').unwrap();
                env.push((k.to_owned(), v.to_owned()));
            }
        } else if let Some(rest) = line.strip_prefix("args:") {
            args = rest.split_whitespace().map(str::to_owned).collect();
        }
    }
    (env, args)
}

fn golden_text(case: &str, file: &str) -> Vec<u8> {
    fs::read(golden_dir(case).join(file)).unwrap()
}

/// The golden's written files: every file in its directory except the five bookkeeping ones.
fn golden_files(case: &str) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = fs::read_dir(golden_dir(case))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !matches!(n.as_str(), "CASE" | "rc" | "stdout.txt" | "stderr.txt"))
        .map(|n| (n.clone(), fs::read(golden_dir(case).join(&n)).unwrap()))
        .collect();
    out.sort();
    out
}

/// The case's own setup: the dirty-tree and graph-edit cases from CAPTURE.md, and the extra brief file.
fn prepare(case: &str, s: &Scratch) {
    match case {
        "allow-dirty-empty" | "allow-dirty-zero" | "dirty-refused" | "dirty-before-agent" => {
            fs::write(s.root.join("junk-untracked"), "").unwrap();
        }
        "docs-edit-and-pin" => s.append("graphs/docs.toml", "# edited\n"),
        "spec-edit-ok" | "spec-edit-ok-true" | "spec-edit-refused" => {
            s.append("graphs/spec.toml", "# edited\n")
        }
        "two-edits-name-first" => {
            s.append("graphs/spec.toml", "# edited\n");
            s.append("graphs/docs.toml", "# edited\n");
        }
        "extra-brief-newlines" => fs::write(
            s.extra.join("extra.md"),
            "Éxtra brief ending with newlines\n\n\n",
        )
        .unwrap(),
        _ => {}
    }
}

/// Runs a golden case in a fresh clone and compares stdout, stderr, the exit code and every written file.
fn check_golden_with(case: &str, opts: &Opts, mutate: impl FnOnce(&Scratch)) -> Scratch {
    let s = Scratch::new(opts);
    prepare(case, &s);
    mutate(&s);
    let (env, args) = read_case(case);
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = s.pii(&env, &args);
    let head = s.head();
    let mut seen = Vec::new();
    let stdout = norm(&out.stdout, &s, &head, &mut seen);
    assert_eq!(
        String::from_utf8_lossy(&stdout),
        String::from_utf8_lossy(&golden_text(case, "stdout.txt")),
        "{case}: stdout"
    );
    if !STDERR_EXCLUDED.contains(&case) {
        let stderr = norm(&out.stderr, &s, &head, &mut seen);
        let want = golden_text(case, "stderr.txt");
        let want = match want.strip_prefix(BASH_RESUME_PREFIX.as_bytes()) {
            Some(rest) => rest.to_vec(),
            None => want,
        };
        assert_eq!(
            String::from_utf8_lossy(&stderr),
            String::from_utf8_lossy(&want),
            "{case}: stderr"
        );
    }
    let rc: i32 = String::from_utf8_lossy(&golden_text(case, "rc"))
        .trim()
        .parse()
        .unwrap();
    assert_eq!(out.status.code(), Some(rc), "{case}: exit code");
    let got = s.files(&mut seen);
    let want = golden_files(case);
    let got_names: Vec<&String> = got.iter().map(|(n, _)| n).collect();
    let want_names: Vec<&String> = want.iter().map(|(n, _)| n).collect();
    assert_eq!(got_names, want_names, "{case}: files written");
    for ((name, body), (_, golden_body)) in got.iter().zip(&want) {
        assert_eq!(
            String::from_utf8_lossy(body),
            String::from_utf8_lossy(golden_body),
            "{case}: {name} bytes"
        );
    }
    assert!(
        seen.windows(2).all(|w| w[0] == w[1]),
        "{case}: TS differs within one run: {seen:?}"
    );
    if rc != 0 {
        assert_eq!(s.logs(), 0, "{case}: an abort must write no log");
    }
    s
}

fn check_golden(case: &str) -> Scratch {
    check_golden_with(case, &Opts::default(), |_| {})
}

fn exact(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[test]
fn spec_lane_matches_the_golden() {
    check_golden("spec-lane");
}

#[test]
fn coder_lane_matches_the_golden() {
    check_golden("coder-lane");
}

/// The dry run cannot reach "not found" (it answers found); the not-found branch is a unit test in `cmd_lane`.
#[test]
fn resume_found_and_not_found_match_their_goldens() {
    check_golden("coder-resume");
    check_golden("spec-resume");
}

#[test]
fn pi_ignores_resume() {
    check_golden("pi-resume-ignored");
    check_golden("pi-coder");
}

#[test]
fn usage_is_on_stdout_exit_2() {
    check_golden("usage");
}

#[test]
fn unknown_spec_picks_the_bytewise_first() {
    let s = Scratch::new(&Opts::default());
    let mut names: Vec<String> = fs::read_dir(s.root.join("specs"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("16-") && n.ends_with(".md"))
        .collect();
    names.sort();
    let first = names.first().expect("a spec 16 exists").clone();
    let out = s.pii(&[], &["16", "--dry-run"]);
    let want = format!("SPEC     : specs/{first}");
    let got = String::from_utf8_lossy(&out.stdout);
    assert!(got.lines().any(|l| l == want), "want {want:?}, got {got}");
}

#[test]
fn three_does_not_match_thirty() {
    check_golden("dash-required");
}

/// The case is named for the spec's 127; the golden and the spec's exit criterion both say 1.
#[test]
fn resume_without_value_exits_127() {
    check_golden("resume-no-value");
}

#[test]
fn dry_run_flag_is_not_a_brief_path() {
    let s = check_golden("coder-lane");
    assert!(!String::from_utf8_lossy(&golden_text("coder-lane", "stderr.txt")).contains("No such"));
    drop(s);
}

#[test]
fn dirty_before_agent_exits_8() {
    check_golden("dirty-before-agent");
}

#[test]
fn allow_dirty_zero_still_allows() {
    check_golden("allow-dirty-zero");
}

#[test]
fn allow_dirty_empty_does_not() {
    check_golden("allow-dirty-empty");
}

#[test]
fn lane_graph_must_be_exactly_spec() {
    check_golden("lane-Spec-is-coder");
}

#[test]
fn edit_ok_must_be_exactly_one() {
    check_golden("spec-edit-ok-true");
}

#[test]
fn wall_check_precedes_head_and_pin() {
    let s = Scratch::new(&Opts::default());
    s.replace("graphs/spec.toml", "max_wall_ms", "max_wall_xx");
    s.append("graphs/docs.toml", "# edited\n");
    let out = s.pii(
        &[("PII_LANE_GRAPH", "spec"), ("ALLOW_DIRTY", "1")],
        &["309", "--dry-run"],
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        exact(&[
            "LANE-GRAPH: spec",
            "ABORT: graphs/spec.toml has no numeric max_wall_ms: nothing started"
        ])
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn two_edited_graphs_name_the_first() {
    check_golden("two-edits-name-first");
}

#[test]
fn staged_edit_is_refused() {
    let s = Scratch::new(&Opts::default());
    s.append("graphs/spec.toml", "# edited\n");
    s.git(&["add", "graphs/spec.toml"]);
    let out = s.pii(
        &[("PII_LANE_GRAPH", "spec"), ("ALLOW_DIRTY", "1")],
        &["309", "--dry-run"],
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        exact(&[
            "LANE-GRAPH: spec",
            "ABORT: graphs/spec.toml differs from HEAD (the pin protects the reviewed graph): nothing started"
        ])
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn mode_only_change_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let s = Scratch::new(&Opts::default());
    let p = s.root.join("graphs/spec.toml");
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    let out = s.pii(
        &[("PII_LANE_GRAPH", "spec"), ("ALLOW_DIRTY", "1")],
        &["309", "--dry-run"],
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        exact(&[
            "LANE-GRAPH: spec",
            "ABORT: graphs/spec.toml differs from HEAD (the pin protects the reviewed graph): nothing started"
        ])
    );
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn ignored_untracked_graph_passes() {
    let opts = Opts {
        ignore: "graphs/docs.toml",
        remove: &[],
    };
    check_golden_with("spec-lane", &opts, |_| {});
}

#[test]
fn missing_preflight_graph_aborts() {
    let opts = Opts {
        ignore: "",
        remove: &["graphs/preflight.toml"],
    };
    let s = Scratch::new(&opts);
    let out = s.pii(&[("PII_LANE_GRAPH", "spec")], &["309", "--dry-run"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        exact(&[
            "LANE-GRAPH: spec",
            "ABORT: graphs/preflight.toml is missing (the preflight judge graph): nothing started"
        ])
    );
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(s.logs(), 0);
}

/// The spec's first `sha256 = "…"` is the pin; a later one is not consulted.
fn pin_case(first_pin: &str) -> String {
    let s = Scratch::new(&Opts::default());
    s.prepend("graphs/spec.toml", &format!("# sha256 = \"{first_pin}\"\n"));
    let body = fs::read(s.root.join("graphs/docs.toml")).unwrap();
    let digest = Sha256::digest(&body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let out = s.pii(
        &[
            ("PII_LANE_GRAPH", "spec"),
            ("ALLOW_DIRTY", "1"),
            ("PII_GRAPH_EDIT_OK", "1"),
        ],
        &["309", "--dry-run"],
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        exact(&[
            "LANE-GRAPH: spec",
            "NOTE: graph edits allowed (PII_GRAPH_EDIT_OK)",
            &format!(
                "ABORT: graphs/docs.toml digest '{digest}' is not the pin in graphs/spec.toml ('{first_pin}'): nothing started"
            ),
        ])
    );
    assert_eq!(out.status.code(), Some(2));
    digest
}

#[test]
fn pin_is_the_first_match() {
    pin_case("0123");
}

#[test]
fn wrong_pin_names_both_digests() {
    let digest = pin_case("abcd");
    assert_eq!(digest.len(), 64);
}

#[test]
fn wall_is_the_first_match() {
    let s = Scratch::new(&Opts::default());
    s.prepend("graphs/spec.toml", "max_wall_ms = 60000\n");
    let out = s.pii(
        &[
            ("PII_LANE_GRAPH", "spec"),
            ("ALLOW_DIRTY", "1"),
            ("PII_GRAPH_EDIT_OK", "1"),
        ],
        &["309", "--dry-run"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout
            .lines()
            .any(|l| l == "WAIT     : up to 660s (graph max_wall_ms 60000 + 600)"),
        "{stdout}"
    );
    assert_eq!(out.status.code(), Some(0));
}

#[test]
fn graph_brief_is_stripped_input_brief_is_not() {
    check_golden("coder-lane");
    check_golden("spec-lane");
}

#[test]
fn coder_budget_default_is_three_raw() {
    check_golden("coder-lane");
    check_golden("coder-budget-env");
}

#[test]
fn budget_capped_at_the_graph_limit() {
    check_golden("spec-budget-capped");
}

#[test]
fn unparseable_budget_truncates_the_input() {
    check_golden("spec-budget-bad");
}

#[test]
fn nan_budget_is_refused() {
    let s = Scratch::new(&Opts::default());
    let out = s.pii(
        &[("PII_LANE_GRAPH", "spec"), ("PII_MAX_BUDGET_USD", "nan")],
        &["309", "--dry-run"],
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        exact(&["LANE-GRAPH: spec", "ABORT: could not build the input"])
    );
    assert_eq!(out.status.code(), Some(2));
    let input = s
        .root
        .join("_tmp/claude-batchC/input-309-a-rows-verify-column-is-its-specs-exit-criteria.json");
    assert_eq!(fs::read(input).unwrap().len(), 0);
}

#[test]
fn a_budget_below_the_limit_is_kept() {
    check_golden("spec-budget-low");
}

#[test]
fn missing_extra_brief_is_not_an_error() {
    check_golden("extra-brief-missing");
}

#[test]
fn triple_quote_leaves_the_brief_file() {
    let s = Scratch::new(&Opts::default());
    fs::write(s.extra.join("q.md"), "a ''' quote\n").unwrap();
    let out = s.pii(&[], &["309", "<SCRATCH>/q.md", "--dry-run"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        exact(&[
            "LANE-GRAPH: coder",
            "ABORT: the brief contains ''' and cannot be written as a TOML literal"
        ])
    );
    assert_eq!(out.status.code(), Some(2));
    let brief = s
        .root
        .join("_tmp/claude-batchC/brief-309-a-rows-verify-column-is-its-specs-exit-criteria.md");
    assert!(
        fs::read_to_string(brief)
            .unwrap()
            .ends_with("a ''' quote\n")
    );
    assert_eq!(s.logs(), 0);
}

#[test]
fn byte_count_counts_bytes() {
    check_golden("extra-brief-newlines");
}

#[test]
fn one_ts_everywhere() {
    check_golden("coder-lane");
    check_golden("spec-lane");
}

#[test]
fn no_log_on_any_abort() {
    for case in [
        "spec-edit-refused",
        "usage",
        "dirty-refused",
        "docs-edit-and-pin",
        "bogus-agent",
    ] {
        check_golden(case);
    }
}

#[test]
fn input_equals_python_json_dumps() {
    let s = check_golden("spec-lane");
    let input = s
        .root
        .join("_tmp/claude-batchC/input-309-a-rows-verify-column-is-its-specs-exit-criteria.json");
    let got = fs::read(input).unwrap();
    assert!(!got.ends_with(b"\n"), "json.dumps has no trailing newline");
    assert!(serde_json::from_slice::<serde_json::Value>(&got).is_ok());
}

/// Every `$.input.<key>` the spec graph reads must be a key of the input, and the input has exactly these keys.
#[test]
fn input_keys_cover_every_graph_key() {
    let graph = fs::read_to_string(live().join("graphs/spec.toml")).unwrap();
    let mut used: Vec<String> = graph
        .split("$.input.")
        .skip(1)
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        })
        .collect();
    used.sort();
    used.dedup();
    let s = Scratch::new(&Opts::default());
    let out = s.pii(&[("PII_LANE_GRAPH", "spec")], &["309", "--dry-run"]);
    assert_eq!(out.status.code(), Some(0));
    let input =
        fs::read(s.root.join(
            "_tmp/claude-batchC/input-309-a-rows-verify-column-is-its-specs-exit-criteria.json",
        ))
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&input).unwrap();
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    let mut want: Vec<String> = [
        "agent",
        "model",
        "provider",
        "spec",
        "base",
        "workdir",
        "brief",
        "docs_brief",
        "max_budget_usd",
        "sid",
    ]
    .iter()
    .map(|k| (*k).to_owned())
    .collect();
    want.sort();
    assert_eq!(keys, want);
    for k in &used {
        assert!(
            keys.contains(k),
            "graph reads input.{k}, which the input lacks"
        );
    }
}

#[test]
fn a_clean_tree_passes() {
    check_golden("coder-lane");
}

#[test]
fn an_unchanged_graph_passes() {
    check_golden("spec-lane");
}

#[test]
fn the_edit_opt_out_prints_its_note() {
    check_golden("spec-edit-ok");
}

/// The live oracle: the real shell and the port, each on its own scratch clone, agree on every golden case.
#[test]
fn matches_the_shell_reference_in_a_scratch_clone() {
    let mut cases: Vec<String> =
        fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/lane"))
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
    cases.sort();
    for case in cases {
        let (env, args) = read_case(&case);
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let shell = Scratch::new(&Opts::default());
        prepare(&case, &shell);
        let port = Scratch::new(&Opts::default());
        prepare(&case, &port);
        let a = shell.shell(&env, &args);
        let b = port.pii(&env, &args);
        let (ha, hb) = (shell.head(), port.head());
        let (mut seen_a, mut seen_b) = (Vec::new(), Vec::new());
        assert_eq!(
            String::from_utf8_lossy(&norm(&b.stdout, &port, &hb, &mut seen_b)),
            String::from_utf8_lossy(&norm(&a.stdout, &shell, &ha, &mut seen_a)),
            "{case}: stdout"
        );
        if !STDERR_EXCLUDED.contains(&case.as_str()) {
            let mut sa = norm(&a.stderr, &shell, &ha, &mut seen_a);
            if let Some(rest) = sa.strip_prefix(b"<SCRIPT>: line 39: 2: ") {
                sa = rest.to_vec();
            }
            let sb = norm(&b.stderr, &port, &hb, &mut seen_b);
            assert_eq!(
                String::from_utf8_lossy(&sb),
                String::from_utf8_lossy(&sa),
                "{case}: stderr"
            );
        }
        assert_eq!(a.status.code(), b.status.code(), "{case}: exit code");
        assert_eq!(
            shell.files(&mut seen_a),
            port.files(&mut seen_b),
            "{case}: files"
        );
        assert!(
            seen_a.windows(2).all(|w| w[0] == w[1]),
            "{case}: shell TS differs: {seen_a:?}"
        );
        assert!(
            seen_b.windows(2).all(|w| w[0] == w[1]),
            "{case}: port TS differs: {seen_b:?}"
        );
    }
}

/// Spec 320 no-compare, measured on the 2026-10-30 fresh-gate run: an export has no .git, so the
/// reviewed-graph pin has no HEAD to differ from — the lane must run, not abort.
#[test]
fn a_no_git_tree_runs_without_the_pin() {
    let s = Scratch::new(&Opts::default());
    fs::remove_dir_all(s.root.join(".git")).unwrap();
    let out = s.pii(
        &[("PII_LANE_GRAPH", "spec"), ("ALLOW_DIRTY", "1")],
        &["309", "--dry-run", "--root", s.root.to_str().unwrap()],
    );
    let out_text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out_text.contains("differs from HEAD"),
        "no HEAD: {out_text}"
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {} | stderr: {}",
        out_text,
        String::from_utf8_lossy(&out.stderr)
    );
}

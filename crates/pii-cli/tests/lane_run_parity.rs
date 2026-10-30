//! Spec 319 acceptance: `pii lane run` against a recording ssh stub. The scratch repos live under
//! `CARGO_TARGET_TMPDIR` (never `_tmp/`, which the port refuses as a root). The stub records every remote argument
//! in order and fails on any argument it does not expect, so an extra or reordered remote call is visible.
//! The lane's children run with a scrubbed environment (spec 319 divergence), so the stub's paths are baked in.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods, // reason: the tests drive git, the shell and the stub directly
    clippy::needless_pass_by_value
)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_pii");
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The stub ssh: records the whole argv and the remote argument, then answers from the files the test wrote. Anything
/// else fails loudly. The preflight judge is `r000000000002` and answers from `PRE_FILE`; the lane's run is
/// `r000000000001` and answers from `STATUS_FILE`. `*_FLAG` files switch a probe to its failing answer.
const SSH_STUB: &str = r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> 'ARGV_PATH'
remote="${*: -1}"
printf '%s\n' "$remote" >> 'REC_PATH'
case "$remote" in
  *"command -v uv"*) if [ -e 'UV_FLAG' ]; then echo "uv: not found"; exit 1; fi; exit 0 ;;
  *"find "*) if [ -e 'MISS_FLAG' ]; then exit 1; fi; exit 0 ;;
  *"wc -l"*) echo 0 ;;
  *"run start preflight"*) echo "started r000000000002" ;;
  *"run start"*) echo "$(< 'START_FILE')" ;;
  *"/start-"*) echo "$(< 'START_FILE')" ;;
  *"coder.spawned"*) echo '{"session":"sid-1"}' ;;
  *"run get r000000000002"*) echo "$(< 'PRE_FILE')" ;;
  *"run get"*) echo "$(< 'STATUS_FILE')" ;;
  *"run cancel"*) echo "cancelled r000000000001" ;;
  *"run explain"*) echo "explained" ;;
  *"run trace"*) echo "crit=c1 fail=none" ;;
  *"coder.exited"*) if [ -e 'EDIT_FILE' ]; then printf 'edited\n' >> 'SPEC_PATH'; fi; echo "$(< 'EXITED_FILE')" ;;
  *) echo "unexpected remote argument: $remote" >&2; exit 1 ;;
esac
"#;

/// The daemon stub: `daemon.sh up` succeeds with a last line. A test that needs it down rewrites it (`fail_daemon`).
const DAEMON_STUB: &str = r#"#!/usr/bin/env bash
echo "daemon up: build abc"
"#;

const SUCCEEDED: &str = r#"{"status":"succeeded"}"#;
const CLEAN_EXIT: &str = r#"{"rc":"0","tree_changed":"no"}"#;
const START: &str = "started r000000000001";

/// The live repository (the shell hard-codes it, so the differential runs against it).
fn live_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// One stub: the ssh and daemon scripts, and the files the answers come from.
struct Stub {
    rec: PathBuf,
    argv: PathBuf,
    status: PathBuf,
    pre: PathBuf,
    start: PathBuf,
    exited: PathBuf,
    edit: PathBuf,
    uv: PathBuf,
    miss: PathBuf,
    ssh: PathBuf,
    daemon: PathBuf,
}

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Stub {
    /// Stub files named by `tag`, in the scratch base. `spec` is the spec the agent would edit.
    fn new(tag: &str, spec: &Path) -> Self {
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let tag = format!("{}-{tag}", std::process::id());
        let file = |kind: &str, ext: &str| base.join(format!("{kind}-{tag}.{ext}"));
        let stub = Self {
            rec: file("rec", "txt"),
            argv: file("argv", "txt"),
            status: file("status", "txt"),
            pre: file("pre", "txt"),
            start: file("start", "txt"),
            exited: file("exited", "txt"),
            edit: file("edit", "flag"),
            uv: file("uv-fail", "flag"),
            miss: file("resume-miss", "flag"),
            ssh: base.join(format!("ssh-stub-{tag}")),
            daemon: base.join(format!("daemon-stub-{tag}")),
        };
        for stale in [&stub.rec, &stub.argv, &stub.edit, &stub.uv, &stub.miss] {
            let _ = fs::remove_file(stale);
        }
        let ssh_body = SSH_STUB
            .replace("ARGV_PATH", &stub.argv.display().to_string())
            .replace("REC_PATH", &stub.rec.display().to_string())
            .replace("STATUS_FILE", &stub.status.display().to_string())
            .replace("PRE_FILE", &stub.pre.display().to_string())
            .replace("START_FILE", &stub.start.display().to_string())
            .replace("EXITED_FILE", &stub.exited.display().to_string())
            .replace("EDIT_FILE", &stub.edit.display().to_string())
            .replace("UV_FLAG", &stub.uv.display().to_string())
            .replace("MISS_FLAG", &stub.miss.display().to_string())
            .replace("SPEC_PATH", &spec.display().to_string());
        executable(&stub.ssh, &ssh_body);
        executable(&stub.daemon, DAEMON_STUB);
        fs::write(&stub.start, START).unwrap();
        fs::write(&stub.pre, SUCCEEDED).unwrap();
        stub.answers(SUCCEEDED, CLEAN_EXIT);
        stub
    }

    /// The lane run's status, and the `coder.exited` lines (one per coder: host first, docs second on the spec lane).
    fn answers(&self, status: &str, exited: &str) {
        fs::write(&self.status, status).unwrap();
        fs::write(&self.exited, exited).unwrap();
    }

    /// The recorded remote arguments, in call order.
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.rec)
            .map(|s| s.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

/// A scratch repo for the binary: one spec (`319`), committed graphs (the live spec-lane graphs, with a one-second
/// wall so the wait expires at once), and a stub.
struct Lane {
    root: PathBuf,
    stub: Stub,
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?} failed");
}

/// The scratch base for a lane root: the port refuses a root with a `_tmp` path component, and the gate's target dir lives
/// under `_tmp/`, so `CARGO_TARGET_TMPDIR` alone fails in the gate. Same order as `lane_plan_parity`: the runtime dir, then
/// `CARGO_TARGET_TMPDIR`, then `/var/tmp`; the first that exists and has no `_tmp` component.
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
        let base = candidate.join("pii-lane319");
        let outside_tmp = !base.components().any(|c| c.as_os_str() == "_tmp");
        if outside_tmp && fs::create_dir_all(&base).is_ok() {
            return base;
        }
    }
    panic!("no scratch base free of a _tmp path component");
}

impl Lane {
    fn new(name: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let base = scratch_base();
        let root = base.join(format!("lane319-{}-{name}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("specs")).unwrap();
        fs::create_dir_all(root.join("graphs")).unwrap();
        fs::write(
            root.join("specs/319-x.md"),
            "# Spec 319\n\n## Exit Criteria\n- [ ] `true`\n",
        )
        .unwrap();
        for g in ["spec", "docs", "preflight"] {
            let text =
                fs::read_to_string(live_root().join("graphs").join(format!("{g}.toml"))).unwrap();
            let text = if g == "spec" {
                text.lines()
                    .map(|l| {
                        if l.starts_with("max_wall_ms") {
                            "max_wall_ms = 1000".to_owned()
                        } else {
                            l.to_owned()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n"
            } else {
                text
            };
            fs::write(root.join("graphs").join(format!("{g}.toml")), text).unwrap();
        }
        git(&root, &["init", "-q"]);
        git(&root, &["add", "specs", "graphs"]);
        git(&root, &["commit", "-qm", "base"]);
        let stub = Stub::new(&format!("{name}-{n}"), &root.join("specs/319-x.md"));
        Self { root, stub }
    }

    /// The agent edits `specs/319-x.md` when its coder exits.
    fn agent_edits_spec(&self) {
        fs::write(&self.stub.edit, "").unwrap();
    }

    /// Makes the daemon stub fail: exit 3 with a message on stderr.
    fn fail_daemon(&self) {
        executable(
            &self.stub.daemon,
            "#!/usr/bin/env bash\necho boom >&2\nexit 3\n",
        );
    }

    /// `pii lane run 319` on the scratch repo. `spec` selects the spec lane; `node_timeout_ms` bounds the coder lane.
    fn run_with(&self, spec: bool, node_timeout_ms: Option<&str>) -> Output {
        self.spawn(spec, node_timeout_ms, &[], &[])
    }

    /// The general spawn: extra arguments after `319`, and extra environment variables.
    fn spawn(
        &self,
        spec: bool,
        node_timeout_ms: Option<&str>,
        extra: &[&str],
        envs: &[(&str, &str)],
    ) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("ALLOW_DIRTY", "1")
            .env("PII_SSH", &self.stub.ssh)
            .env("PII_DAEMON_SH", &self.stub.daemon)
            .env("PII_POLL_SECONDS", "0")
            .env("PII_WAIT_MARGIN_S", "0")
            // The scratch repo's HEAD is a fixture commit, not the binary's build ref: the stale-
            // binary guard is about production runs, and its own tests live in lane_binary_label.rs.
            .env("PII_STALE_BIN_OK", "1")
            .args(["lane", "run", "319"])
            .args(extra)
            .args(["--root"])
            .arg(&self.root)
            .args(["--agent-root", "/projects/pi-improved"]);
        if spec {
            cmd.env("PII_LANE_GRAPH", "spec");
        }
        if let Some(ms) = node_timeout_ms {
            cmd.env("PII_NODE_TIMEOUT_MS", ms);
        }
        for (k, v) in envs {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }

    fn run(&self, node_timeout_ms: Option<&str>) -> Output {
        self.run_with(false, node_timeout_ms)
    }

    fn calls(&self) -> Vec<String> {
        self.stub.calls()
    }
}

fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn position(calls: &[String], needle: &str) -> usize {
    calls
        .iter()
        .position(|c| c.contains(needle))
        .unwrap_or_else(|| panic!("no remote call containing {needle:?} in {calls:#?}"))
}

/// One recorded call with the run-specific parts normalised: the repo path, the graph name's and judge input's
/// timestamps, and the preflight repo's timestamp and random suffix (`<TS>`, `<RAND>`, spec 319 divergences).
fn normalise(call: &str, root: &str) -> String {
    let line = call
        .replace(root, "<REPO>")
        .replace("/projects/pi-improved", "<REPO>");
    let mut out = String::new();
    let mut rest = line.as_str();
    let markers = ["q319-", "preflight-", "judge-input-"];
    while let Some((i, marker)) = markers
        .iter()
        .filter_map(|m| rest.find(m).map(|i| (i, *m)))
        .min()
    {
        out.push_str(&rest[..i + marker.len()]);
        rest = &rest[i + marker.len()..];
        rest = &rest[rest.bytes().take_while(u8::is_ascii_digit).count()..];
        out.push_str("<TS>");
        if marker == "preflight-" && rest.starts_with('-') {
            rest = &rest[1..];
            out.push('-');
            rest = &rest[rest.bytes().take_while(u8::is_ascii_alphanumeric).count()..];
            out.push_str("<RAND>");
        }
    }
    out.push_str(rest);
    out
}

/// The differential (spec 319 Oracle): the composed shell and the binary, each against its own stub that gives the
/// same answers, on the live repo. Both make the same remote calls in the same order and exit with the same code.
fn differential(spec: bool, status: &str, exited: &str) {
    let live = live_root();
    let live_text = live.display().to_string();
    let spec_file = live.join("specs/319-x.md");
    let n = NEXT.fetch_add(1, Ordering::SeqCst);

    let shell_stub = Stub::new(&format!("shell-{n}"), &spec_file);
    shell_stub.answers(status, exited);
    let mut shell = Command::new("bash");
    shell
        .arg(live.join("scripts/run-batchC-pii.sh"))
        .arg("319")
        .current_dir(&live)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("ALLOW_DIRTY", "1")
        .env("PII_SSH", &shell_stub.ssh)
        .env("PII_DAEMON_SH", &shell_stub.daemon)
        .env("PII_POLL_SECONDS", "0")
        // uv's default cache is under /root, which this sandbox cannot write; the cache is scratch either way.
        .env(
            "UV_CACHE_DIR",
            PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("uv-cache"),
        );
    if spec {
        shell.env("PII_LANE_GRAPH", "spec");
    }
    let shell = shell.output().unwrap();

    let bin_stub = Stub::new(&format!("bin-{n}"), &spec_file);
    bin_stub.answers(status, exited);
    let mut bin = Command::new(BIN);
    bin.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("ALLOW_DIRTY", "1")
        .env("PII_SSH", &bin_stub.ssh)
        .env("PII_DAEMON_SH", &bin_stub.daemon)
        .env("PII_POLL_SECONDS", "0")
        // Spec 321 R3: wherever this suite runs (the CI-model export lives under _tmp/), both halves
        // must read the same tree — the binary half takes the deliberate-root knob.
        .env("PII_LANE_TMP_ROOT_OK", "1")
        .args([
            "lane",
            "run",
            "319",
            "--root",
            &live_text,
            "--agent-root",
            "/projects/pi-improved",
        ]);
    if spec {
        bin.env("PII_LANE_GRAPH", "spec");
    }
    let bin = bin.output().unwrap();

    let bin_err = String::from_utf8_lossy(&bin.stderr);
    assert_eq!(
        bin.status.code(),
        shell.status.code(),
        "exit codes differ\nshell: {}\nbinary: {}\nbinary stderr: {}",
        text(&shell),
        text(&bin),
        bin_err,
    );
    let want: Vec<String> = shell_stub
        .calls()
        .iter()
        .map(|c| normalise(c, &live_text))
        .collect();
    let got: Vec<String> = bin_stub
        .calls()
        .iter()
        .map(|c| normalise(c, &live_text))
        .collect();
    assert!(!want.is_empty(), "the shell made no remote call");
    assert_eq!(
        got, want,
        "the recorded ssh sequence differs from the shell's"
    );
}

#[test]
fn daemon_down_aborts_before_anything() {
    let lane = Lane::new("daemon-down");
    lane.fail_daemon();
    let out = lane.run(None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(2), "{stdout}");
    assert!(
        stdout.contains("ABORT: the devbox daemon is not up:"),
        "{stdout}"
    );
    assert!(
        lane.calls().is_empty(),
        "no remote call may run: {:?}",
        lane.calls()
    );
}

#[test]
fn success_exits_0_with_facts() {
    let lane = Lane::new("success");
    let out = lane.run(None);
    let stdout = text(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("RUN      : r000000000001"), "{stdout}");
    assert!(
        stdout.contains("== session handle captured at spawn: sid-1"),
        "{stdout}"
    );
    assert!(
        stdout
            .contains("FACTS    : rc=0 tree_changed=no quota_exhausted=? cost_usd=not-journalled"),
        "{stdout}"
    );
    assert!(stdout.contains("tree after:"), "{stdout}");
}

#[test]
fn a_successful_run_has_no_quota_lines() {
    let lane = Lane::new("no-quota");
    let stdout = text(&lane.run(None));
    assert!(
        !stdout.contains("QUOTA"),
        "a successful run has no quota lines: {stdout}"
    );
}

#[test]
fn the_recorded_ssh_sequence_equals_the_shells() {
    // Every scenario runs both implementations against the same stub answers.
    differential(false, SUCCEEDED, CLEAN_EXIT);
    differential(true, SUCCEEDED, CLEAN_EXIT);
    differential(
        true,
        r#"{"status":"failed"}"#,
        r#"{"rc":"1","quota_exhausted":"yes","quota_reset":"5:30pm"}"#,
    );
    differential(
        true,
        r#"{"status":"failed"}"#,
        "{\"rc\":\"0\",\"tree_changed\":\"no\"}\n{\"rc\":\"1\",\"quota_exhausted\":\"yes\",\"quota_reset\":\"9:05am\"}",
    );
}

#[test]
fn session_handle_is_captured_and_written() {
    let lane = Lane::new("session-file");
    let out = lane.run(None);
    assert_eq!(out.status.code(), Some(0));
    let logd = lane.root.join("_tmp/claude-batchC");
    let found = fs::read_dir(&logd)
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().starts_with("session-319"))
        .expect("session-<tag>.txt written");
    assert_eq!(fs::read_to_string(found.path()).unwrap(), "sid-1\n");
}

#[test]
fn wait_expiry_cancels_with_kill_and_the_reason() {
    let lane = Lane::new("wait-expiry");
    lane.stub.answers(r#"{"status":"running"}"#, CLEAN_EXIT);
    let out = lane.run_with(true, None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    let calls = lane.calls();
    position(
        &calls,
        "run cancel r000000000001 --kill --reason lane-wait-expired",
    );
    assert!(
        stdout.contains("== r000000000001 still running after "),
        "{stdout}"
    );
    assert!(stdout.contains("s: cancelled by the lane"), "{stdout}");
    assert!(
        stdout.contains("== why the run was not accepted:"),
        "{stdout}"
    );
}

#[test]
fn failure_exits_1_with_the_why_lines() {
    let lane = Lane::new("failure");
    lane.stub.answers(
        r#"{"status":"failed"}"#,
        r#"{"rc":"1","tree_changed":"no"}"#,
    );
    let out = lane.run_with(true, None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains(r#"final: "status":"failed" after ~0s"#),
        "{stdout}"
    );
    assert!(
        stdout.contains("== why the run was not accepted:"),
        "{stdout}"
    );
    assert!(stdout.contains("crit=c1 fail=none"), "{stdout}");
}

#[test]
fn host_quota_refusal_prints_the_reset_time() {
    let lane = Lane::new("quota-reset");
    lane.stub.answers(
        r#"{"status":"failed"}"#,
        r#"{"rc":"1","quota_exhausted":"yes","quota_reset":"5:30pm"}"#,
    );
    let stdout = text(&lane.run(None));
    assert!(
        stdout.contains("You've hit your session limit · resets 5:30pm (UTC)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("QUOTA    : refused by the host coder"),
        "{stdout}"
    );
}

#[test]
fn docs_quota_refusal_names_the_docs_coder() {
    let lane = Lane::new("quota-docs");
    lane.stub.answers(
        r#"{"status":"failed"}"#,
        "{\"rc\":\"0\",\"tree_changed\":\"no\"}\n{\"rc\":\"1\",\"quota_exhausted\":\"yes\",\"quota_reset\":\"9:05am\"}",
    );
    let stdout = text(&lane.run_with(true, None));
    assert!(
        stdout.contains("You've hit your session limit · resets 9:05am (UTC)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("QUOTA    : refused by the docs coder"),
        "{stdout}"
    );
    assert!(
        !stdout.contains("refused by the host coder"),
        "the host coder did not refuse: {stdout}"
    );
}

#[test]
fn refusal_without_a_reset_time_prints_the_bare_line() {
    let lane = Lane::new("quota-bare");
    lane.stub.answers(
        r#"{"status":"failed"}"#,
        r#"{"rc":"1","quota_exhausted":"yes"}"#,
    );
    let stdout = text(&lane.run(None));
    assert!(
        stdout.contains("\nYou've hit your session limit\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("resets"), "{stdout}");
    assert!(
        stdout.contains("QUOTA    : refused by the host coder"),
        "{stdout}"
    );
}

#[test]
fn facts_quote_stripping_matches_the_shell() {
    let lane = Lane::new("facts");
    lane.stub.answers(
        SUCCEEDED,
        r#"{"rc":0,"tree_changed":"yes","claude_cost_usd":"0.42"}"#,
    );
    let stdout = text(&lane.run(None));
    assert!(
        stdout.contains("FACTS    : rc=0 tree_changed=yes quota_exhausted=? cost_usd=0.42"),
        "{stdout}"
    );
}

#[test]
fn coder_lane_polls_are_bounded_by_the_node_timeout() {
    let lane = Lane::new("bounded");
    lane.stub.answers(r#"{"status":"running"}"#, CLEAN_EXIT);
    let out = lane.run(Some("5000"));
    assert_eq!(out.status.code(), Some(1));
    // max = 5000 / 5000 + 120 polls, and nothing else polls
    let polls = lane
        .calls()
        .iter()
        .filter(|c| c.contains("run get"))
        .count();
    assert_eq!(polls, 121);
}

#[test]
fn agent_edit_under_specs_exits_9_and_lists_it() {
    let lane = Lane::new("spec-edit");
    lane.agent_edits_spec();
    let out = lane.run(None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(9), "{stdout}");
    assert!(
        stdout.contains("SPEC-GUARD: the agent changed files under specs/"),
        "{stdout}"
    );
    assert!(stdout.contains("specs/319-x.md"), "{stdout}");
}

#[test]
fn tree_after_lists_at_most_12_paths() {
    let lane = Lane::new("tree");
    for i in 0..15 {
        fs::write(lane.root.join(format!("junk-{i:02}.txt")), "x").unwrap();
    }
    let stdout = text(&lane.run(None));
    let listed = stdout.lines().filter(|l| l.starts_with("?? ")).count();
    assert_eq!(listed, 12, "{stdout}");
    assert!(stdout.contains("== tree after: "), "{stdout}");
}

#[test]
fn the_spec_guard_passes_when_specs_are_untouched() {
    let lane = Lane::new("guard-clean");
    let out = lane.run(None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(!stdout.contains("SPEC-GUARD"), "{stdout}");
}

#[test]
fn daemon_is_called_through_pii_daemon_sh() {
    // The daemon is named, not ported: the binary calls the configured script and nothing else starts the devbox.
    let lane = Lane::new("daemon-named");
    let out = lane.run(None);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(text(&out).contains("DAEMON   : daemon up: build abc"));
}

#[test]
fn preflight_failure_prints_the_judges_trace_lines() {
    let lane = Lane::new("preflight-fail");
    fs::write(&lane.stub.pre, r#"{"status":"failed"}"#).unwrap();
    let out = lane.run_with(true, None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(2), "{stdout}");
    assert!(
        stdout.contains("ABORT: container preflight failed: the daemon's judge cannot write"),
        "{stdout}"
    );
    assert!(stdout.contains("crit=c1 fail=none"), "{stdout}");
    position(&lane.calls(), "run trace r000000000002");
    assert!(
        !lane.calls().iter().any(|c| c.contains("/start-")),
        "the run must not start after a failed preflight"
    );
}

#[test]
fn no_run_id_aborts_with_300_bytes() {
    let lane = Lane::new("no-run-id");
    fs::write(&lane.stub.start, "x".repeat(400)).unwrap();
    let out = lane.run(None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(2), "{stdout}");
    let abort = stdout
        .lines()
        .find(|l| l.starts_with("ABORT: no run id: "))
        .expect("the no-run-id abort line");
    assert_eq!(abort, format!("ABORT: no run id: {}", "x".repeat(300)));
}

#[test]
fn has_uv_failure_aborts_with_the_probe_output() {
    let lane = Lane::new("has-uv");
    fs::write(&lane.stub.uv, "").unwrap();
    let out = lane.run_with(true, None);
    let stdout = text(&out);
    assert_eq!(out.status.code(), Some(2), "{stdout}");
    assert!(
        stdout.contains("ABORT: container preflight failed: uv: not found"),
        "{stdout}"
    );
    position(&lane.calls(), "command -v uv >/dev/null");
    assert!(
        !lane.calls().iter().any(|c| c.contains("/start-")),
        "nothing starts when uv is missing"
    );
}

#[test]
fn resume_found_and_not_found_via_the_real_devbox_impl() {
    let lane = Lane::new("resume-found");
    let out = lane.spawn(false, None, &["--resume", "sid-x"], &[]);
    let stdout = text(&out);
    assert!(
        stdout.contains("== RESUMING claude session sid-x"),
        "{stdout}"
    );
    let calls = lane.calls();
    assert!(
        calls.iter().any(|c| c.contains("podman exec")
            && c.contains("-name \"sid-x.jsonl\" -type f -print -quit")),
        "{calls:#?}"
    );

    let missing = Lane::new("resume-missing");
    fs::write(&missing.stub.miss, "").unwrap();
    let out = missing.spawn(false, None, &["--resume", "sid-x"], &[]);
    let stdout = text(&out);
    assert!(
        stdout.contains("RESUME: session sid-x not found in the devbox, starting fresh"),
        "{stdout}"
    );
}

#[test]
fn ssh_binary_and_host_come_from_the_environment() {
    let lane = Lane::new("env-host");
    let out = lane.spawn(
        false,
        None,
        &[],
        &[("DEVBOX_HOST", "lane-user@devbox.example")],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let argv = fs::read_to_string(&lane.stub.argv).unwrap();
    let first = argv.lines().next().expect("the stub recorded a call");
    assert!(
        first.starts_with("-o BatchMode=yes -o ConnectTimeout=15 lane-user@devbox.example "),
        "{first}"
    );
}

#[test]
fn no_hardcoded_host_in_the_source() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cmd_lane");
    let mut hits = Vec::new();
    for entry in fs::read_dir(&dir).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        let body = fs::read_to_string(&path).unwrap();
        for line in body.lines() {
            if line.contains("10.10.10.217") {
                hits.push(format!("{}: {line}", path.display()));
            }
        }
    }
    let [hit] = hits.as_slice() else {
        panic!("the default host lives in one constant: {hits:#?}");
    };
    assert!(
        hit.contains("env.rs") && hit.contains("DEFAULT_HOST"),
        "{hit}"
    );
}

// ---- spec 321: the _tmp/ refusal keeps its default and gains an exactly-1 knob ----

/// A lane root under a path component literally named `_tmp`, per spec 321 (fresh-gate exports).
fn tmp_named_root(tag: &str) -> PathBuf {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lane321-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let root = base.join("_tmp").join("lane-root");
    fs::create_dir_all(&root).unwrap();
    root
}

fn lane(args: &[&str], knob: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_pii"));
    c.args(["lane", "plan"]).args(args).env_clear().env("PATH", "/usr/bin:/bin");
    if let Some(k) = knob {
        c.env("PII_LANE_TMP_ROOT_OK", k);
    }
    c.output().unwrap()
}

/// R1: without the knob, the refusal is byte-for-byte: stderr names the root, stdout is empty, rc 2.
#[test]
fn a_tmp_root_is_refused_without_the_knob() {
    let root = tmp_named_root("refused");
    let o = lane(&["1", "--root", root.to_str().unwrap()], None);
    assert_eq!(o.status.code(), Some(2));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains(&format!("pii lane: {} is inside _tmp/: refused", root.display())), "{err}");
    assert!(o.stdout.is_empty(), "refusal prints nothing on stdout");
}

/// R2: with the knob exactly `1`, planning continues and the NOTE (with the root) hits stdout first.
#[test]
fn a_tmp_root_runs_with_the_knob_and_says_so() {
    let root = tmp_named_root("knobbed");
    let o = lane(&["1", "--root", root.to_str().unwrap()], Some("1"));
    // The run may fail later (no git repo in the scratch), but it MUST get past the refusal: no
    // `is inside _tmp/` line, and the NOTE printed on stdout.
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!err.contains("is inside _tmp/"), "refusal must not fire with the knob: {err}");
    let out = String::from_utf8_lossy(&o.stdout);
    let note = format!("NOTE: the lane root {} is inside _tmp/ (PII_LANE_TMP_ROOT_OK=1)", root.display());
    assert!(out.contains(&note), "NOTE names the root: {out}\n{err}");
}

/// R2 exactness control: `0`, `01`, `true` and empty do NOT open the hatch.
#[test]
fn the_tmp_knob_needs_exactly_1() {
    for bad in ["0", "01", "true", ""] {
        let root = tmp_named_root(&format!("exact-{bad}"));
        let o = lane(&["1", "--root", root.to_str().unwrap()], Some(bad));
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "knob {bad:?} must not open the hatch");
        assert!(err.contains("is inside _tmp/: refused"), "knob {bad:?}: {err}");
    }
}

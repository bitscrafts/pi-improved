//! The lane's recording scaffold (spec 319, shared by spec 320): a scratch repo, the ssh and daemon stubs, and `pii lane run`
//! driven against them. Moved here from `lane_run_parity.rs` unchanged except for `pub` and the default `PII_STALE_BIN_OK=1`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::disallowed_methods, // reason: the tests drive git, the shell and the stub directly
    clippy::needless_pass_by_value
)]
#![allow(dead_code, clippy::redundant_pub_crate)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
pub(crate) const BIN: &str = env!("CARGO_BIN_EXE_pii");
pub(crate) static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The stub ssh: records the whole argv and the remote argument, then answers from the files the test wrote. Anything
/// else fails loudly. The preflight judge is `r000000000002` and answers from `PRE_FILE`; the lane's run is
/// `r000000000001` and answers from `STATUS_FILE`. `*_FLAG` files switch a probe to its failing answer.
pub(crate) const SSH_STUB: &str = r#"#!/usr/bin/env bash
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
pub(crate) const DAEMON_STUB: &str = r#"#!/usr/bin/env bash
echo "daemon up: build abc"
"#;

pub(crate) const SUCCEEDED: &str = r#"{"status":"succeeded"}"#;
pub(crate) const CLEAN_EXIT: &str = r#"{"rc":"0","tree_changed":"no"}"#;
pub(crate) const START: &str = "started r000000000001";

/// The live repository (the shell hard-codes it, so the differential runs against it).
pub(crate) fn live_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// One stub: the ssh and daemon scripts, and the files the answers come from.
pub(crate) struct Stub {
    pub(crate) rec: PathBuf,
    pub(crate) argv: PathBuf,
    pub(crate) status: PathBuf,
    pub(crate) pre: PathBuf,
    pub(crate) start: PathBuf,
    pub(crate) exited: PathBuf,
    pub(crate) edit: PathBuf,
    pub(crate) uv: PathBuf,
    pub(crate) miss: PathBuf,
    pub(crate) ssh: PathBuf,
    pub(crate) daemon: PathBuf,
}

pub(crate) fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

impl Stub {
    /// Stub files named by `tag`, in the scratch base. `spec` is the spec the agent would edit.
    pub(crate) fn new(tag: &str, spec: &Path) -> Self {
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
    pub(crate) fn answers(&self, status: &str, exited: &str) {
        fs::write(&self.status, status).unwrap();
        fs::write(&self.exited, exited).unwrap();
    }

    /// The recorded remote arguments, in call order.
    pub(crate) fn calls(&self) -> Vec<String> {
        fs::read_to_string(&self.rec)
            .map(|s| s.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }
}

/// A scratch repo for the binary: one spec (`319`), committed graphs (the live spec-lane graphs, with a one-second
/// wall so the wait expires at once), and a stub.
pub(crate) struct Lane {
    pub(crate) root: PathBuf,
    pub(crate) stub: Stub,
}

pub(crate) fn git(dir: &Path, args: &[&str]) {
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
pub(crate) fn scratch_base() -> PathBuf {
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
    pub(crate) fn new(name: &str) -> Self {
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
    pub(crate) fn agent_edits_spec(&self) {
        fs::write(&self.stub.edit, "").unwrap();
    }

    /// Makes the daemon stub fail: exit 3 with a message on stderr.
    pub(crate) fn fail_daemon(&self) {
        executable(
            &self.stub.daemon,
            "#!/usr/bin/env bash\necho boom >&2\nexit 3\n",
        );
    }

    /// `pii lane run 319` on the scratch repo. `spec` selects the spec lane; `node_timeout_ms` bounds the coder lane.
    pub(crate) fn run_with(&self, spec: bool, node_timeout_ms: Option<&str>) -> Output {
        self.spawn(spec, node_timeout_ms, &[], &[])
    }

    /// The general spawn: extra arguments after `319`, and extra environment variables.
    pub(crate) fn spawn(
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
            // A scratch repo never has the binary's commit as HEAD (spec 320): the stale-binary guard is set aside here.
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

    pub(crate) fn run(&self, node_timeout_ms: Option<&str>) -> Output {
        self.run_with(false, node_timeout_ms)
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.stub.calls()
    }
}

pub(crate) fn text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

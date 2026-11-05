//! The run half (spec 319 R2-R6): `scripts/run-batchC-pii.sh` lines 219-343 as one driver over an injected transport,
//! clock and host effects. It returns stdout, log appends, files and the exit code; the thin shell writes them.
use super::LanePlan;
use super::clock::Clock;
use super::env::LaneEnv;
use super::facts::{fact, quoted, reset_time};
use super::guard;
use super::preflight::{PreflightRepo, judge_input};
use super::transport::{Out, Transport};
use super::wait::{is_terminal, run_id, session, status};
use std::path::{Path, PathBuf};

/// Everything the run produces. Nothing here touches the host.
#[derive(Debug, Default)]
pub(crate) struct RunReport {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    /// Appended to the lane's log, in order (the shell's `>> "$LOG"` and `tee -a`).
    pub(crate) log_appends: Vec<u8>,
    /// Written in order.
    pub(crate) files: Vec<(PathBuf, Vec<u8>)>,
    pub(crate) exit: i32,
}

/// The host effects the run needs, injected so tests script them.
pub(crate) struct Io<'a> {
    pub(crate) t: &'a dyn Transport,
    pub(crate) clock: &'a dyn Clock,
    /// `bash $PII_DAEMON_SH up`, stderr merged into stdout.
    pub(crate) daemon: &'a dyn Fn() -> Out,
    /// `git status --porcelain -- specs`, raw.
    pub(crate) specs_status: &'a dyn Fn() -> Vec<u8>,
    /// `git status --porcelain`, raw.
    pub(crate) status: &'a dyn Fn() -> Vec<u8>,
    /// Makes the throwaway judge repo, or fails with the ABORT line to print.
    pub(crate) preflight_repo: &'a dyn Fn() -> Result<PreflightRepo, String>,
    /// The spec lane's copies of the three graphs into the devbox-test graphs dir.
    pub(crate) graph_copies: Vec<(PathBuf, Vec<u8>)>,
}

/// The paths the shell's variables name: `c` is `$C` (the devbox-test dir), `agent_root` is the devbox's view of the
/// repo (`podman exec -w`), `logd` is `$LOGD` and `scratch` is `$REPO/_tmp`.
pub(crate) struct Paths {
    pub(crate) c: String,
    pub(crate) agent_root: String,
    pub(crate) logd: PathBuf,
    pub(crate) scratch: String,
}

/// Runs the lane's second half. `node_timeout_ms` is the raw `PII_NODE_TIMEOUT_MS` (decimal, checked by the plan).
pub(crate) fn run(
    plan: &LanePlan,
    env: &LaneEnv,
    paths: &Paths,
    io: &Io<'_>,
    node_timeout_ms: u64,
) -> RunReport {
    let poll = env
        .poll_seconds
        .as_deref()
        .map_or(5, |raw| raw.parse().unwrap_or(0));
    let mut d = Driver {
        io,
        paths,
        container: env
            .container
            .clone()
            .unwrap_or_else(|| "pii-devbox".to_owned()),
        piib: format!("{}/bin/pii", paths.c),
        poll,
        r: RunReport::default(),
    };
    d.lane(plan, env.spec_lane, node_timeout_ms);
    d.r
}

struct Driver<'a> {
    io: &'a Io<'a>,
    paths: &'a Paths,
    container: String,
    piib: String,
    poll: u64,
    r: RunReport,
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The lines of `printf '%s\n' "$(…)"`: trailing newlines dropped, an empty capture is one empty line.
fn echo_lines(bytes: &[u8]) -> Vec<String> {
    let text = lossy(bytes);
    text.trim_end_matches('\n')
        .split('\n')
        .map(str::to_owned)
        .collect()
}

/// The lines of a plain command's output (`grep`, `head`): none for empty output.
fn raw_lines(bytes: &[u8]) -> Vec<String> {
    lossy(bytes).lines().map(str::to_owned).collect()
}

fn or<'a>(value: &'a str, default: &'a str) -> &'a str {
    if value.is_empty() { default } else { value }
}

fn why_line(line: &str) -> bool {
    line.contains("crit=") || line.contains("fail=")
}

impl Driver<'_> {
    fn say(&mut self, line: impl AsRef<str>) {
        self.r.stdout.extend_from_slice(line.as_ref().as_bytes());
        self.r.stdout.push(b'\n');
    }

    fn log(&mut self, line: impl AsRef<str>) {
        self.r
            .log_appends
            .extend_from_slice(line.as_ref().as_bytes());
        self.r.log_appends.push(b'\n');
    }

    /// `tee -a "$LOG"`: the line to stdout and to the log.
    fn tee(&mut self, line: impl AsRef<str>) {
        self.say(line.as_ref());
        self.log(line);
    }

    /// `podman exec -w <agent_root> <container> <args>`: the shell's `X`.
    fn x(&self, args: &str) -> String {
        format!(
            "podman exec -w {} {} {args}",
            self.paths.agent_root, self.container
        )
    }

    /// `sh -c 'env PII_RUNTIME_DIR=<C>/run <pii> <args> 2>&1<tail>'`: the composed argument of `X`.
    fn pii(&self, args: &str, tail: &str) -> String {
        format!(
            "sh -c 'env PII_RUNTIME_DIR={}/run {} {args} 2>&1{tail}'",
            self.paths.c, self.piib
        )
    }

    /// A remote call whose ssh stderr reaches the lane's stderr (the shell does not redirect it).
    fn call(&mut self, remote: &str) -> Out {
        let out = self.io.t.exec(remote);
        self.r.stderr.extend_from_slice(&out.stderr);
        out
    }

    /// A remote call whose ssh stderr the shell discards (`2>/dev/null`).
    fn quiet(&self, remote: &str) -> Out {
        self.io.t.exec(remote)
    }

    /// One `run get` poll: the first `"status":"…"` pair, or empty.
    fn poll_status(&mut self, rid: &str) -> String {
        let remote = self.x(&self.pii(&format!("run get {rid}"), ""));
        status(&lossy(&self.call(&remote).stdout))
    }

    /// The lines `wc -l` counted before the start (`LINES_BEFORE`); unreadable is zero.
    fn lines_before(&self) -> usize {
        let remote = format!(
            "podman exec {} sh -c 'wc -l < {}/pii.log'",
            self.container, self.paths.c
        );
        lossy(&self.quiet(&remote).stdout)
            .replace(' ', "")
            .trim()
            .parse()
            .unwrap_or(0)
    }

    fn lane(&mut self, plan: &LanePlan, spec: bool, node_timeout: u64) {
        let logd = self.paths.logd.clone();
        let tag = plan.tag.clone();
        let c = self.paths.c.clone();

        // ---- the daemon, up and its last line stated
        let d = (self.io.daemon)();
        let text = echo_lines(&d.stdout);
        self.say(format!(
            "DAEMON   : {}",
            text.last().map_or("", String::as_str)
        ));
        if d.code != 0 {
            self.say("ABORT: the devbox daemon is not up:");
            for line in text.iter().skip(text.len().saturating_sub(5)) {
                self.say(line);
            }
            self.r.exit = 9; // refused before anything was spent (spec 397), as the shell lane exits
            return;
        }
        if spec {
            self.r.files.extend(self.io.graph_copies.iter().cloned());
        }

        // ---- the spec snapshot before anything runs, then the judge preflight (spec lane)
        let before = guard::sorted(&(self.io.specs_status)());
        self.r.files.push((
            logd.join(format!(".specs-before-{tag}")),
            guard::render(&before),
        ));
        if spec && !self.preflight(&logd) {
            return;
        }

        // ---- the start: the spec lane runs a generated script, the coder lane a graph by name
        let lb = self.lines_before();
        let start = if spec {
            let input = plan
                .input
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let script = logd.join(format!("start-{tag}.sh"));
            let body = format!(
                "#!/bin/sh\nenv PII_RUNTIME_DIR={c}/run {} run start spec --input \"$(cat {input})\" 2>&1\n",
                self.piib
            );
            self.r.files.push((script.clone(), body.into_bytes()));
            let remote = self.x(&format!("sh {}", script.display()));
            self.call(&remote)
        } else {
            let remote = self.x(&self.pii(&format!("run start {}", plan.graph_name), ""));
            self.call(&remote)
        };
        let Some(rid) = run_id(&lossy(&start.stdout)) else {
            let head: Vec<u8> = start.stdout.iter().take(300).copied().collect();
            self.say(format!(
                "ABORT: no run id: {}",
                lossy(&head).trim_end_matches('\n')
            ));
            self.r.exit = 9; // refused before anything was spent (spec 397), as the shell lane exits
            return;
        };
        self.say(format!("RUN      : {rid}"));
        self.log(format!("== run {rid}"));

        // ---- the session handle, the moment pii journals it (coder.spawned)
        let mut sid = String::new();
        for _ in 0..60 {
            let remote = format!(
                "podman exec {} sh -c 'tail -n +{} {c}/pii.log | grep -a coder.spawned | tail -1'",
                self.container,
                lb + 1
            );
            sid = session(&lossy(&self.quiet(&remote).stdout));
            if !sid.is_empty() {
                break;
            }
            self.io.clock.sleep(2);
        }
        if !sid.is_empty() {
            self.r.files.push((
                logd.join(format!("session-{tag}.txt")),
                format!("{sid}\n").into_bytes(),
            ));
            self.say(format!("== session handle captured at spawn: {sid}"));
        }

        // ---- the wait
        let mut st = String::new();
        let waited = if spec {
            let t0 = self.io.clock.now();
            let deadline = plan.deadline_s.unwrap_or(0);
            loop {
                st = self.poll_status(&rid);
                if is_terminal(&st) || self.io.clock.now() - t0 >= deadline {
                    break;
                }
                self.io.clock.sleep(self.poll);
            }
            if !is_terminal(&st) {
                let remote = self.x(&self.pii(
                    &format!("run cancel {rid} --kill --reason lane-wait-expired"),
                    "",
                ));
                let canc = self.call(&remote);
                let elapsed = self.io.clock.now() - t0;
                self.tee(format!(
                    "== {rid} still running after {elapsed}s: cancelled by the lane"
                ));
                for line in echo_lines(&canc.stdout) {
                    self.tee(line);
                }
            }
            self.io.clock.now() - t0
        } else {
            let max = node_timeout / 5000 + 120;
            let mut i = 0;
            for k in 1..=max {
                i = k;
                st = self.poll_status(&rid);
                if is_terminal(&st) {
                    break;
                }
                self.io.clock.sleep(self.poll);
            }
            i * self.poll
        };
        let shown = or(&st, "unknown").to_owned();
        self.tee(format!("== {rid} final: {shown} after ~{waited}s"));

        // ---- the evidence: explain (first 60 lines) into the log, and on a refused spec run the criteria
        let explain = self.quiet(&self.x(&self.pii(&format!("run explain {rid}"), " | head -60")));
        for line in raw_lines(&explain.stdout).into_iter().take(60) {
            self.log(line);
        }
        self.r.log_appends.extend_from_slice(&explain.stderr);
        if spec && !st.contains("succeeded") {
            let trace = self.call(&self.x(&self.pii(&format!("run trace {rid}"), "")));
            let why: Vec<String> = raw_lines(&trace.stdout)
                .into_iter()
                .filter(|l| why_line(l))
                .collect();
            if why.is_empty() {
                self.tee("== why the run was not accepted: (the trace names no criterion)");
            } else {
                self.tee("== why the run was not accepted:");
                for line in why {
                    self.tee(line);
                }
            }
        }
        let exits_remote = format!(
            "podman exec {} sh -c 'tail -n +{} {c}/pii.log | grep -a coder.exited'",
            self.container,
            lb + 1
        );
        let exits = echo_lines(&self.quiet(&exits_remote).stdout);
        let (exited, docs_exited) = if spec {
            (
                exits.first().cloned().unwrap_or_default(),
                exits.get(1).cloned().unwrap_or_default(),
            )
        } else {
            (exits.last().cloned().unwrap_or_default(), String::new())
        };
        self.log(format!("== coder.exited: {exited}"));
        self.facts(spec, &exited, &docs_exited);

        // ---- specs are read-only for the worker: the result is checked, not trusted
        let after = guard::sorted(&(self.io.specs_status)());
        self.r.files.push((
            logd.join(format!(".specs-after-{tag}")),
            guard::render(&after),
        ));
        let touched = guard::touched(&before, &after);
        if !touched.is_empty() {
            self.tee(
                "SPEC-GUARD: the agent changed files under specs/ (read-only for implementers):",
            );
            for line in touched {
                self.tee(line);
            }
            self.r.exit = 9;
            return;
        }
        let tree = lossy(&(self.io.status)());
        self.say(format!(
            "== tree after: {} dirty paths",
            tree.lines().count()
        ));
        for line in tree.lines().take(12) {
            self.say(line);
        }
        self.r.exit = i32::from(!st.contains("succeeded"));
    }

    /// The FACTS, DOCS-FACTS and QUOTA lines (spec 319 R5): the first coder's exit, the docs coder's on the spec lane.
    fn facts(&mut self, spec: bool, exited: &str, docs: &str) {
        let (rc, chg) = (fact(exited, "rc"), fact(exited, "tree_changed"));
        let qx = fact(exited, "quota_exhausted");
        let cost = quoted(exited, "claude_cost_usd");
        let (mut qby, mut qreset) = ("host", quoted(exited, "quota_reset"));
        if spec && qx != "yes" && fact(docs, "quota_exhausted") == "yes" {
            qby = "docs";
            qreset = quoted(docs, "quota_reset");
        }
        self.say(format!(
            "FACTS    : rc={} tree_changed={} quota_exhausted={} cost_usd={}",
            or(&rc, "?"),
            or(&chg, "?"),
            or(&qx, "?"),
            or(&cost, "not-journalled")
        ));
        if spec {
            let dcost = quoted(docs, "claude_cost_usd");
            self.say(format!(
                "DOCS-FACTS: rc={} tree_changed={} quota_exhausted={} cost_usd={}",
                fact(docs, "rc"),
                fact(docs, "tree_changed"),
                fact(docs, "quota_exhausted"),
                or(&dcost, "not-journalled")
            ));
        }
        if qx == "yes" || qby == "docs" {
            match reset_time(&qreset) {
                Some(t) => self.tee(format!("You've hit your session limit · resets {t} (UTC)")),
                None => self.tee("You've hit your session limit"),
            }
            self.tee(format!("QUOTA    : refused by the {qby} coder"));
        }
    }

    /// The judge preflight (spec 301, `S:234-255`). False when it aborted the run (exit 2 is set).
    fn preflight(&mut self, logd: &Path) -> bool {
        let pre = match (self.io.preflight_repo)() {
            Ok(p) => p,
            Err(msg) => {
                self.say(msg);
                self.r.exit = 9; // refused before anything was spent (spec 397), as the shell lane exits
                return false;
            }
        };
        let workdir = format!("{}/{}/repo", self.paths.c, pre.dir);
        let input = logd.join(format!("judge-input-{}.json", pre.ts));
        self.r
            .files
            .push((input.clone(), judge_input(&pre.rev, &workdir).into_bytes()));
        let remote = self.x(&self.pii(
            &format!("run start preflight --input \"$(cat {})\"", input.display()),
            "",
        ));
        let start = self.call(&remote);
        let Some(prid) = run_id(&lossy(&start.stdout)) else {
            self.say("ABORT: the preflight judge did not start: nothing started");
            self.r.exit = 9; // refused before anything was spent (spec 397), as the shell lane exits
            return false;
        };
        let mut pst = String::new();
        for _ in 0..60 {
            pst = self.poll_status(&prid);
            if is_terminal(&pst) {
                break;
            }
            self.io.clock.sleep(self.poll);
        }
        if pst.contains("succeeded") {
            self.say(format!(
                "PREFLIGHT: the daemon's judge wrote {} ok ({prid})",
                self.paths.scratch
            ));
            return true;
        }
        self.say(format!(
            "ABORT: container preflight failed: the daemon's judge cannot write {} ({prid}: {})",
            self.paths.scratch,
            or(&pst, "no status")
        ));
        let trace = self.call(&self.x(&self.pii(&format!("run trace {prid}"), "")));
        for line in raw_lines(&trace.stdout).iter().filter(|l| why_line(l)) {
            self.say(line);
        }
        self.r.exit = 9; // refused before anything was spent (spec 397), as the shell lane exits
        false
    }
}

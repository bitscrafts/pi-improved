//! Spec 185 (observability): the coder journals the route its agent reports. A child that writes its
//! session header and exits at once must still have that header captured: the capture is finished only after
//! the child exits, so a fast exit cannot lose the output. The child runs through the real capability path
//! (`Ctx::proc()` -> `spawn_child_exact`), the same one `wait_child` uses. Control: a silent child captures nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::{Duration, Instant};

use pii_core::{ArgvScope, Budget, CapKind, CapSet, Ctx};
use pii_server::node_kinds::nodes::coder_runtime::StdoutCapture;

/// Runs `/bin/sh -c <script>` through the proc capability, captures its stdout the way `wait_child` does
/// (start the capture, wait for the child, then finish), and returns the captured text.
fn run_captured(script: &str) -> String {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![String::from("/bin/sh")]);
    let ctx = Ctx::root(caps, Budget::unlimited());
    let argv = vec![
        String::from("/bin/sh"),
        String::from("-c"),
        script.to_owned(),
    ];
    let env = vec![(String::from("PATH"), String::from("/usr/bin:/bin"))];
    let mut child = ctx
        .proc()
        .unwrap()
        .spawn_child_exact(&argv, Path::new("/"), &env)
        .unwrap();
    let capture = StdoutCapture::start(child.stdout());
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if child
            .wait_timeout(Duration::from_millis(20))
            .unwrap()
            .is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline, "the child did not exit");
    }
    capture.finish()
}

#[test]
fn a_fast_exiting_child_has_its_output_captured() {
    let out = run_captured(
        "echo '{\"type\":\"session\",\"provider\":\"ollama-cloud\",\"modelId\":\"m\"}'",
    );
    assert!(
        out.contains("\"provider\":\"ollama-cloud\""),
        "the header of a run that exits at once is not lost: {out:?}"
    );
}

#[test]
fn a_silent_child_captures_nothing() {
    let out = run_captured("true");
    assert!(
        out.is_empty(),
        "a child with no output captures nothing: {out:?}"
    );
}

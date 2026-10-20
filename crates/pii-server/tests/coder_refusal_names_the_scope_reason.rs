//! Spec 223 R2-R4: a `coder` node that the run scope refuses before any spawn journals `worker.start.refused` with the
//! scope's own message, and the image's digest slot reads `missing`. The run is REFUSED, never FAILED.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use pii_abi::{CapKind, PiiError, RunStatus};
use pii_core::{ArgvScope, Budget, CapSet, Ctx};
use pii_graph::NodeRegistry;
use pii_observe::{Event, Observer, names};
use pii_server::node_kinds::nodes::coder::CoderFactory;

#[derive(Default)]
struct Rec(Mutex<Vec<Event>>);

impl Observer for Rec {
    fn event(&self, e: &Event) {
        self.0.lock().push(e.clone());
    }
}

/// A fake podman that would run anything: a refusal must stop before it is ever executed.
fn fake_podman(dir: &Path) -> std::path::PathBuf {
    let log = dir.join("podman.log");
    let script = format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 0\n", log.display());
    let podman = dir.join("podman");
    std::fs::write(&podman, script).unwrap();
    std::fs::set_permissions(&podman, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    podman
}

/// The scope admits `/bin/sh` and the agent but not podman, so the image probe is denied by the scope.
#[test]
fn scope_refusal_names_its_reason_and_the_run_is_refused() {
    let bin = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-bin-").unwrap();
    let podman = fake_podman(bin.path());
    let work = pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-work-").unwrap();
    let store_dir =
        pii_store::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "coder-store-").unwrap();
    let store = pii_store::Store::open(&store_dir.path().join("store.redb")).unwrap();
    let log = Arc::new(Rec::default());
    let mut reg = NodeRegistry::builtin();
    reg.register(Arc::new(
        CoderFactory::with_env(bin.path().to_path_buf(), BTreeMap::new(), log.clone())
            .with_podman(podman),
    ));
    let src = format!(
        "name = \"coder-refused\"\nentry = \"code\"\ncaps = [\"spawn\", \"proc\"]\n\n[[node]]\nid = \"code\"\nkind = \"coder\"\ncaps = [\"spawn\", \"proc\"]\nagent = \"pi\"\nmodel = \"m\"\nbrief = \"b\"\nspec = \"specs/223-x.md\"\nworkdir = \"{}\"\ncontainer_image = \"pii-coder:1\"\n\n[[node]]\nid = \"refused\"\nkind = \"transform\"\nvalue = \"refused\"\n\n[[edge]]\nfrom = \"code\"\nport = \"err\"\nto = \"refused\"\nkind = \"data\"\n",
        work.path().display()
    );
    let spec = pii_graph::load_toml_with_registry(&src, &reg).unwrap();
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::SPAWN | CapKind::PROC;
    caps.proc_argv0 = ArgvScope::new(vec![
        String::from("/bin/sh"),
        bin.path().join("pi").display().to_string(),
    ]);
    let run_id = store.next_run_id().unwrap();
    pii_graph::run::run_with_validate(
        &store,
        &spec,
        &reg,
        &Ctx::root(caps, Budget::unlimited()),
        &run_id,
        serde_json::json!({}),
        true,
    )
    .unwrap();

    let status = store.get_run(&run_id).unwrap().expect("stored").status;
    assert_eq!(
        status,
        RunStatus::Refused,
        "a scope refusal is a refused run"
    );
    assert!(
        !bin.path().join("podman.log").exists(),
        "a refused probe never executes podman"
    );
    let events = log.0.lock().clone();
    assert!(!events.iter().any(|e| e.kind == names::CODER_SPAWNED));
    let refused = events
        .iter()
        .find(|e| e.kind == names::WORKER_START_REFUSED)
        .unwrap_or_else(|| {
            panic!(
                "no worker.start.refused: {:?}",
                events.iter().map(|e| e.kind).collect::<Vec<_>>()
            )
        });
    let reason = format!("{:?}", refused.fields.get("reason"));
    let denial = PiiError::Denied(String::from("proc")).to_string();
    assert!(
        reason.contains(&denial),
        "reason names the scope: {reason} vs {denial}"
    );
    // Spec 231 R2: refused before the runtime answered, so the slot is `probe-failed`, never `missing`.
    let digest = format!("{:?}", refused.fields.get("image_digest"));
    assert!(
        digest.contains("probe-failed"),
        "no digest was probed: {digest}"
    );
    assert!(
        !digest.contains("missing"),
        "a refusal before the runtime is not an absent image: {digest}"
    );
}

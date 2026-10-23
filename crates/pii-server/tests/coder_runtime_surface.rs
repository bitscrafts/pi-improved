//! The container-runtime surface specs 241-243 call, named through its public path (spec 245 R2): a stub implementor
//! that overrides `run` with a timeout, `list_all` and `force_remove`, and a call to `pii_core::kill_group`. Dropping
//! any of them from a commit breaks this file's build, so the caller's side fails, not only the callee's.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::time::Duration;

use pii_core::{Budget, CapSet, Ctx, PiiError, kill_group};
use pii_server::node_kinds::nodes::coder_runtime::{
    ContainerRuntime, Launch, Listed, Request, Spawn,
};

struct Stub;

impl ContainerRuntime for Stub {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn run(
        &self,
        _ctx: &Ctx,
        _req: &Request<'_>,
        timeout: Option<Duration>,
        spawned: &mut dyn FnMut(&Spawn),
    ) -> Result<Launch, PiiError> {
        spawned(&Spawn {
            runtime: "stub",
            image: String::new(),
            image_digest: String::new(),
            container: String::from("c1"),
            pid: u32::try_from(timeout.map_or(0, |t| t.as_secs())).unwrap(),
            argv: Vec::new(),
            env_names: Vec::new(),
        });
        Err(PiiError::Internal(String::from("stub")))
    }

    fn list_all(&self, _ctx: &Ctx) -> Result<Vec<Listed>, PiiError> {
        Ok(vec![Listed {
            id: String::from("c1"),
            labels: vec![(String::from("run"), String::from("r1"))],
        }])
    }

    fn force_remove(&self, _ctx: &Ctx, id: &str) -> Result<bool, PiiError> {
        Ok(id == "c1")
    }
}

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

#[test]
fn the_stub_answers_what_the_trait_promises() {
    let rt = Stub;
    let ctx = ctx();
    let listed = rt.list_all(&ctx).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "c1");
    assert_eq!(listed[0].labels[0].1, "r1");
    assert!(rt.force_remove(&ctx, "c1").unwrap());
    assert!(!rt.force_remove(&ctx, "gone").unwrap());

    let argv: Vec<String> = Vec::new();
    let req = Request {
        image: "i",
        workdir: "/w",
        network: "none",
        run_id: "r1",
        skills_dir: None,
        agent_argv: &argv,
        env: &[],
    };
    let mut seen = Vec::new();
    let out = rt.run(&ctx, &req, Some(Duration::from_secs(7)), &mut |s| {
        seen.push((s.container.clone(), s.pid));
    });
    assert!(out.is_err());
    assert_eq!(seen, vec![(String::from("c1"), 7)]);
}

#[test]
fn kill_group_is_public_and_refuses_nothing_it_does_not_own() {
    // a pid that cannot be a live group: the call must return, not panic, through the public path
    assert!(kill_group(i32::MAX as u32).is_err());
}

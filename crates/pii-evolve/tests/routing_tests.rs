//! R4: explicit step -> tier -> model routing, model diversity, and the
//! audit log.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_core::{Budget, CapSet, Ctx};
use pii_evolve::routing::{RouteLog, RoutePolicy, Tier};

fn ctx() -> Ctx {
    Ctx::root(CapSet::empty(), Budget::unlimited())
}

fn policy() -> RoutePolicy {
    let mut policy = RoutePolicy::new();
    policy.assign_step("find_signal", Tier::Localize);
    policy.assign_step("write_patch", Tier::Patch);
    policy.assign_step("review_patch", Tier::Review);
    policy.assign_step("survey_tech", Tier::Research);
    policy.assign_step("plan_cycle", Tier::Orchestrator);
    policy.set_model(Tier::Localize, "scout-fast");
    policy.set_model(Tier::Patch, "worker-strong-a");
    policy.set_model(Tier::Review, "worker-strong-b");
    policy.set_model(Tier::Research, "planner");
    policy.set_model(Tier::Orchestrator, "boss");
    policy
}

#[test]
fn route_resolves_step_to_tier() {
    let policy = policy();
    let ctx = ctx();
    let cases = [
        ("find_signal", Tier::Localize, "scout-fast"),
        ("write_patch", Tier::Patch, "worker-strong-a"),
        ("review_patch", Tier::Review, "worker-strong-b"),
        ("survey_tech", Tier::Research, "planner"),
        ("plan_cycle", Tier::Orchestrator, "boss"),
    ];
    for (step, tier, model) in cases {
        let decision = policy.resolve(step, &ctx).expect("resolve");
        assert_eq!(decision.tier, tier);
        assert_eq!(decision.model, model);
    }
}

#[test]
fn review_uses_different_model_than_patch() {
    let policy = policy();
    let ctx = ctx();
    let patch = policy.resolve("write_patch", &ctx).expect("resolve patch");
    let review = policy
        .resolve("review_patch", &ctx)
        .expect("resolve review");
    assert_ne!(
        patch.model, review.model,
        "review must use a different model than patch"
    );
}

#[test]
fn route_log_captures_decision() {
    let policy = policy();
    let ctx = ctx();
    let mut log = RouteLog::new();
    let expected = [
        ("find_signal", Tier::Localize, "scout-fast"),
        ("write_patch", Tier::Patch, "worker-strong-a"),
        ("review_patch", Tier::Review, "worker-strong-b"),
    ];
    for (step, _, _) in expected {
        log.route(&policy, step, &ctx).expect("route");
    }
    assert_eq!(log.entries().len(), expected.len());
    for (entry, (step, tier, model)) in log.entries().iter().zip(expected) {
        assert_eq!(entry.step, step);
        assert_eq!(entry.tier, tier);
        assert_eq!(entry.model, model);
    }
}

# Spec: binding a model role builds a provider that can make a call
#: depends=423 status=pending block=-

## Overview
Row 423 taught the agent node to resolve `model = "@<role>"` through the operator's `[models.<role>]` table and to
refuse an unknown role by name. It resolves the role correctly, but it builds the provider with
`pii_model::provider_from_spec(&provider_field)`, and that function cannot build a networked provider:

```rust
"llamacpp-server" | "openai" | "anthropic" | "gemini" => Err(PiiError::Invalid(format!(
    "{kind}: needs a transport — call {kind}::provider_from_env or resolve_with_transport"
))),
```

Worse, the role's provider field carries the *kind* alone (`provider = "openai"`), and a spec with no `:` is rejected as
`unknown model spec 'openai'`. So for every networked role the node fails at build time, before any model call — the
local `llamacpp-server` role and the cloud `openai` role alike.

Row 423's own tests could not see this: they use a `scripted:<path>` provider, which needs no transport. That is the
false-green this spec exists to close: a check that passes while the live path is broken is worse than no check.

Measured 2026-11-13 on the live daemon: with row 423 in the tree, the `github-issue` planner (role `@plan`) cannot bind
its provider, so a `ready` issue still ends in the refusal branch.

## Requirements
- R1 Binding a role builds its provider the way the daemon builds any other networked provider — `resolve_with_transport`
  (or the equivalent entry point), honouring `OPENAI_BASE_URL`, `ANTHROPIC_BASE_URL`, `GEMINI_BASE_URL` and the
  configured `[llama] host` — so a role whose provider is `openai`, `anthropic`, `gemini` or `llamacpp-server` yields a
  provider that can make a call.
- R2 The transport (or a resolver closure that builds providers) is injected where `AgentFactory` is constructed, so there
  is ONE way providers are built in this process and a test can supply a stub instead of the network.
- R3 An unknown role is still refused by name, naming the node and listing the configured roles (row 423 R2, unchanged).
- R4 A role whose provider cannot be built fails naming the role and the provider kind, before any model call.
- R5 An explicit model id (no `@`) keeps today's behaviour, including the shared provider.
- R6 No change to the coder node's provider construction; it already goes through the working path.
- R7 `crates/pii-server/src/lib.rs` may not grow past its recorded 669 lines; `node_kinds/agent/tests.rs` may not grow
  past 758; `tests/model_tiers.rs` may not grow past 498.

## Architecture
```
AgentFactory { provider, models, transport }        <-- transport added here (R2)
        |
   bind_role(cfg, "@plan")
        |
   resolve_role -> (model, kind = "openai")
        |
   resolve_with_transport("openai:glm-5.3", transport, llama_host)   <-- today: provider_from_spec("openai") = Err
        |
   RoleProvider { role, inner }  -> a provider that can call
```

## TDD Contract
| Test Name | Input | Expected Output |
|---|---|---|
| `a_role_whose_provider_is_networked_binds` | role `plan = (glm-5.3, openai)`, a stub transport | the node builds, and the stub records a request naming `glm-5.3` |
| `a_role_built_provider_reads_the_base_url` | `OPENAI_BASE_URL` set to a stub URL in the env seam, role `provider = "openai"` | the recorded request went to that host |
| `CONTROL_a_role_whose_provider_cannot_be_built_names_the_role` | role `provider = "nonsense"` | the failure names `@plan` and `nonsense`; no model call |
| `CONTROL_an_unknown_role_is_still_refused` | empty role table, node `@plan` | refused by name (row 423 behaviour kept) |

## Exit Criteria
### New in this row
Verified to FAIL before the change: the first two tests below cannot pass while `bind_role` calls `provider_from_spec`.
- [ ] `cargo test -q -p pii-server --test agent_node_model_roles 2>&1 | grep -qE 'test result: ok. [7-9] passed'`

## Guardrails
- Do NOT weaken row 423's tests; extend them (R3/R5 keep their behaviour).
- Do NOT call `provider_from_spec` with a bare provider kind anywhere (R1).
- Do NOT build a second way to make providers: use the transport entry point (R2).
- Do NOT edit the coder node (R6).
- Do NOT exceed the three line baselines (R7). If a file is at its cap, extract rather than grow.

## Files
- `crates/pii-server/src/engine/registry.rs`
- `crates/pii-server/src/node_kinds/agent/mod.rs`
- `crates/pii-server/src/node_kinds/agent/ports_tests.rs`
- `crates/pii-server/src/node_kinds/agent/tests.rs`
- `crates/pii-server/src/observed_provider.rs`
- `crates/pii-server/tests/agent_node_model_roles.rs`
- `crates/pii-server/tests/cancellation_brake.rs`
- `crates/pii-server/tests/fail_port.rs`
- `crates/pii-server/tests/failover_graph.rs`
- `crates/pii-server/tests/model_routing.rs`
- `crates/pii-server/tests/model_tiers.rs`
- `crates/pii-server/tests/review_graph/recording.rs`

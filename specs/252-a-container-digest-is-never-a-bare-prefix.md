# 252 — a container digest is never a bare prefix

## Overview

`coder.spawned.image_digest` is journalled as the literal string `sha256:` with no hex after it when the coder node
runs through the runtime API socket. Measured on lnx 2026-10-24T12:24Z, run `r000000000324`:

```
"runtime":"socket","container":"f35f32c7…","image":"localhost/pii-agent:1",
"image_digest":"sha256:"
```

Spec 232 R4 promised "the digest the runtime will actually run, read before anything starts … never a silent empty
digest". The socket path breaks that promise in the one place it cannot be seen from a test: it builds the value as

```rust
String::from("sha256:")
    + value.pointer("/Digest").and_then(Value::as_str).unwrap_or_default().trim_start_matches("sha256:")
```

and the podman **compat** reply to `GET /images/<ref>/json` has **no `/Digest` key at all**. Measured on the same
host, same socket, the reply's 2655 bytes carry only these digest-shaped keys:

```
KEYS-WITH-DIGEST: ['RepoDigests']
Digest= None
Id= sha256:162ab4b23885f9344ee15534dc7673f11adeb4a00dd2f67f9f4a3e533e27950f
RepoDigests= ['localhost/pii-agent@sha256:2684dee9d0fab97d03fac3f90e7fb774ede93297dbb52b5507142f337d06e7b8']
```

(`/v4.0.0/libpod/images/<ref>/json` does carry `Digest`; the code does not call that route.)

The consequences, both measured or direct from the code:

1. `pii run trace` prints `image=-` for a container that really did start from a named image, because the trace
   formats an 8-character slice of the digest and there is nothing after the prefix.
2. The CLI runtime in the same crate refuses this state instead of hiding it —
   `coder_runtime_cli.rs`: `if digest.is_empty() { Refusal::Image { reason: "image has no digest; pin it by digest" } }`.
   Two paths in one crate therefore disagree about what an image with no readable digest means, and the socket path
   is the one that ships containers.

A test can assert `image_digest.trim_start_matches("sha256:").len() == 64` and be written against the wrong field:
the defect is a **defaulted** field, so the meaningful red is "the function that reads the reply never returns
`Some("")`, and a reply with no digest shape is refused".

## Requirements

- **R1.** One pure function reads a digest out of an inspect reply: `image_digest_from_inspect(reply: &Value, image:
  &str) -> Option<String>`, in `crates/pii-server/src/node_kinds/nodes/coder_runtime_api.rs`. The reply shape is the
  measured one above.
- **R2.** The value it returns is a full digest: `sha256:` followed by exactly 64 lowercase hex characters, with the
  prefix appearing once (a reply that already spells `sha256:<hex>` is normalised, not doubled).
- **R3.** The source order is: the `RepoDigests` entry that names `image` (the part after `@`); else the single
  `RepoDigests` entry when there is exactly one; else `Id` when it starts with `sha256:`; else `None`.
- **R4.** `None` is a **refusal**, not an empty field: the socket path returns the same `Refusal::Image` the CLI path
  returns, with the reason `image has no digest; pin it by digest`. `coder.spawned` is never journalled carrying a
  bare prefix.
- **R5.** A reply that carries `Digest` (the libpod route) keeps working: that key wins when it holds 64 hex.
- **R6.** `cargo fmt` and `cargo clippy` stay at zero warnings on `pii-server`; no file grows past the 450-line gate.

## TDD Contract

Write `crates/pii-server/tests/image_digest_is_never_a_bare_prefix.rs` **first**, with the four cases below, and run
it against the current tree: every case that expects a digest must FAIL on the prefix concatenation, and the refusal
case must fail too (an empty string is returned today, not a refusal). Quote the failing output before touching
`coder_runtime_api.rs`. Then implement R1–R5 and show the same binary green, with the test count moving, not just
the verdict.

Case fixtures come from this spec's measured reply, not from a hand-written object:

| case | reply | expected |
|---|---|---|
| `repo_digest_is_used` | `RepoDigests: ["localhost/pii-agent@sha256:2684dee9…"]`, `Id: sha256:162ab4b2…` | `Some("sha256:2684dee9…")` — the repo digest, not the image id |
| `reply_without_digest_uses_the_image_id` | `RepoDigests: []`, `Id: sha256:162ab4b2…` | `Some("sha256:162ab4b2…")` |
| `no_repo_digest_and_no_id_is_none` | `{}` | `None` — and through the node, a `Refusal::Image` naming `image has no digest` |
| `a_prefixed_digest_is_not_doubled` | `Digest: "sha256:2684dee9…"` | `Some("sha256:2684dee9…")`, exactly one prefix |

## Exit Criteria

- [ ] `cargo test -q -p pii-server --test image_digest_is_never_a_bare_prefix repo_digest_is_used 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test image_digest_is_never_a_bare_prefix reply_without_digest_uses_the_image_id 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test image_digest_is_never_a_bare_prefix no_repo_digest_and_no_id_is_none 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `cargo test -q -p pii-server --test image_digest_is_never_a_bare_prefix a_prefixed_digest_is_not_doubled 2>&1 | grep -qE "test result: ok\. [1-9]"`
- [ ] `grep -q 'fn image_digest_from_inspect' crates/pii-server/src/node_kinds/nodes/coder_runtime_api.rs`
- [ ] `! grep -q 'String::from("sha256:")' crates/pii-server/src/node_kinds/nodes/coder_runtime_api.rs`

The live half — one redeploy to lnx and one coder run whose `coder.spawned` carries `image_digest` with 64 hex — is
recorded in `## Closure` by the orchestrator. It is not a checkbox here because this host has no podman and the plan's
verify runs here; a criterion that cannot run on the host it gates is how spec 227's criterion 7 sat unmeasured for
four days.

## Guardrails

- A worker that finds the reply shape different from this spec's measured sample STOPS and says so with the bytes it
  saw; it does not "adapt" the parse to a guess and it does not touch `specs/`.
- Do not weaken a test, and do not fix this by deleting the `image=-` column from `pii run trace`: the trace is
  reading the journal correctly; the field is what is wrong.
- Do not change the CLI runtime's refusal text or behaviour — the socket path is brought to it, not the reverse.
- The refusal must name the requested image, so an operator reading `run get` knows which reference to pin.
- Scratch under `/projects/_tmp/`, never `/tmp`; no `rm -rf`; no commit.

## Closure

(outcome, the four test names with their counts, the mutation that reddens the parse and the `sha256sum` before and
after the restore, the gate line, and the live `coder.spawned` line from lnx with the 64-hex digest — filled in when
the item is closed.)

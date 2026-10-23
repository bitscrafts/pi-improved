#!/usr/bin/env bash
# Independent verification of spec 246 against its WHOLE Exit Criteria list, plus two mutations, inside the devbox. No worker active.
#   M1 revert the wiring in graph_node.rs only: the NODE-level test must go red (the pure-function tests must stay green: they do not
#      see the wiring, which is exactly why the node-level test exists).
#   M2 make brief_with_spec return the brief unchanged: the pure tests must go red.
# Both files are restored byte-identically (sha256).
cd /projects/pi-improved || exit 2
if ps -eo args | grep -q '[c]laude -p'; then echo "worker active: not mutating the tree"; exit 3; fi
ssh -o BatchMode=yes mvcorrea@10.10.10.217 'bash -s' <<'REMOTE'
X() { podman exec -w /projects/pi-improved pii-devbox "$@"; }
G=crates/pii-server/src/node_kinds/nodes/coder/graph_node.rs; P=crates/pii-server/src/node_kinds/nodes/coder/prompt.rs
T=coder_brief_names_the_spec
echo "== c1 the spec's tests"; X sh -c "cargo test -q -p pii-server --test $T 2>&1 | grep -E '^test result|FAILED'"
echo "== c2 clippy -D warnings (pii-server)"; X sh -c "cargo clippy -q -p pii-server --all-targets -- -D warnings 2>&1 | head -5; echo clippy rc=\$?"
echo "== c3 the neighbour still passes"; X sh -c "cargo test -q -p pii-server --test coder_run_needs_its_events 2>&1 | grep -E '^test result'"
echo "== c4 file-size lint"; X sh -c "cargo run -q -p xtask -- lint-files 2>&1 | tail -1"
echo "== c5 the rest of the coder suite (nothing the change could have broken)"
X sh -c "for t in coder_no_change coder_quota_signal coder_stop_reason agent_claude_arm agent_claude_session_handle coder_output_redacted coder_agent_timeout; do printf '%s: ' \$t; cargo test -q -p pii-server --test \$t 2>&1 | grep -E '^test result' | cut -c1-48; done"
echo "== M1: wiring reverted (prompt -> brief), node-level test must go RED"
X sh -c "sha256sum $G > /tmp/g.sha; cp $G /tmp/g.orig; sed -i 's/plan(&planned, &prompt, &env/plan(\&planned, \&brief, \&env/' $G; grep -c 'plan(&planned, &brief' $G; cargo test -q -p pii-server --test $T 2>&1 | grep -E '^test result|^test .*FAILED' | head -4; cp /tmp/g.orig $G; sha256sum -c /tmp/g.sha"
echo "== M2: brief_with_spec returns the brief unchanged, pure tests must go RED"
X sh -c "sha256sum $P > /tmp/p.sha; cp $P /tmp/p.orig; sed -i 's/^    format!(\$/    return brief.to_owned(); #[allow(unreachable_code)] format!(/' $P; cargo test -q -p pii-server --test $T 2>&1 | grep -E '^test result|^test .*FAILED|^error' | head -6; cp /tmp/p.orig $P; sha256sum -c /tmp/p.sha"
REMOTE

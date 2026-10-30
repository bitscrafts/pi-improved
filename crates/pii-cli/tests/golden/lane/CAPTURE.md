# How these goldens were captured (spec 317)
Reference: scripts/run-batchC-pii.sh at commit e3c361c. Procedure (repeat it to re-capture):
1. git clone the repository to a scratch directory; call it R.
2. Copy the script and rewrite its two hardcoded roots: the line starting REPO= (S:21) to REPO=R and the line starting C= (S:24) to C=R/_tmp/devbox-test.
3. Run each case from R with DEVBOX_HOST=nobody@127.0.0.1 and the env and args in the case's CASE file. stdout and stderr are kept apart; rc is the exit code.
4. Every file the script wrote under R/_tmp/claude-batchC and R/_tmp/devbox-test/graphs is copied into the case directory, then moved aside so the next case starts clean.
5. Normalise with exact strings: R -> <REPO>, the clone's HEAD sha -> <HEAD>; and one regex, a ten-digit number -> <TS> (a test must also assert every <TS> in one run is equal).
Dirty-tree cases add an untracked file to R; graph cases append a comment line to graphs/spec.toml and/or graphs/docs.toml, then restore with git checkout.
Cases run: allow-dirty-empty allow-dirty-zero bogus-agent coder-budget-env coder-lane coder-resume dash-required dirty-before-agent dirty-refused docs-edit-and-pin extra-brief-missing extra-brief-newlines lane-Spec-is-coder pi-coder pi-resume-ignored resume-no-value spec-budget-bad spec-budget-capped spec-budget-low spec-edit-ok spec-edit-ok-true spec-edit-refused spec-lane spec-resume two-edits-name-first unknown-spec usage 

#!/usr/bin/env bash
# gate-env.sh — the queue's gate selector (spec 262). Sourced by cycle.sh.
#
# A queue item's commit is gated with the FULL list (gate-steps.txt), whatever GATE_STEPS holds in the environment.
# env.sh exports the quick list for its own callers, and a queue that read GATE_STEPS would commit under three steps
# while the log said the gate passed. So this selector never reads GATE_STEPS.

# gate_steps_file: the list (relative to the repo root) that a queue item's commit is gated with.
gate_steps_file(){
  echo gate-steps.txt
}

# gate_step_labels <file>: the label of each step in a gate list, one per line. Blank and # lines have no label.
gate_step_labels(){
  sed -n 's/^\([^#|][^|]*\)|.*/\1/p' "$1"
}

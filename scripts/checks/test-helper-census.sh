#!/usr/bin/env bash
# Census for spec 147: test-helper duplication per crate.
# Usage: test-helper-census.sh   (prints the command, one row per crate, the rule's outcome; exit 0)
# Counted helpers = test fns that REIMPLEMENT a shared concern: scratch/tmp dirs (a body that does not
# delegate to pii_store::testing::tmp_scratch), store fixtures, fake clocks (not pii_core FixedClock users),
# event capture (not pii_observe Recorder users). Delegating wrappers are listed as "wrap", not counted.
# Rule: build a shared testkit crate iff >= 2 crates outside pii-store each duplicate >= 3 helpers.
set -u
root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root" || exit 2
pat_scratch='fn (scratch|scratch_dir|tmp_dir|tmp_path|temp_dir|temp_path|temp_store|tmp_store)[[:space:]]*[(<]'
pat_store='fn (fresh_store|fresh_mem_store|open_test_store|test_store|mem_store)[[:space:]]*[(<]'
pat_clock='(struct|impl [A-Za-z]+ for) (Fake|Mock|Test)[A-Za-z]*Clock|fn (fake_clock|test_clock)[[:space:]]*[(<]'
pat_capture='struct (Capture|EventCapture|EventSink|EventRecorder|Collecting)[A-Za-z]*|fn (capture_events|collect_events|event_capture)[[:space:]]*[(<]'
echo "command: grep -rEh -A6 '<pattern>' crates/<crate> --include=*.rs, per category (patterns in this script); a scratch fn whose next 6 lines mention tmp_scratch is a wrapper"
# own(): matching lines in a crate whose 6-line body window does not contain tmp_scratch
own() { grep -rE -A6 "$2" "$1" --include=*.rs 2>/dev/null | awk -v p="$2" '
  /^--$/ { if (hit && !wrap) n++; hit=0; wrap=0; next }
  { if ($0 ~ p) { if (hit && !wrap) n++; hit=1; wrap=0 } if ($0 ~ /tmp_scratch/) wrap=1 }
  END { if (hit && !wrap) n++; print n+0 }'; }
wrap() { grep -rE -A6 "$2" "$1" --include=*.rs 2>/dev/null | grep -c 'tmp_scratch'; }
printf '%-16s %8s %6s %6s %8s %6s %5s\n' crate scratch store clock capture total wrap
fires=0
for d in crates/*/; do
  c="$(basename "$d")"
  a=$(own "$d" "$pat_scratch"); b=$(own "$d" "$pat_store")
  k=$(own "$d" "$pat_clock"); e=$(own "$d" "$pat_capture"); w=$(wrap "$d" "$pat_scratch")
  t=$((a + b + k + e))
  [ "$t" -gt 0 ] || [ "$w" -gt 0 ] || continue
  printf '%-16s %8d %6d %6d %8d %6d %5d\n' "$c" "$a" "$b" "$k" "$e" "$t" "$w"
  if [ "$c" != pii-store ] && [ "$t" -ge 3 ]; then fires=$((fires + 1)); fi
done
echo "crates outside pii-store with >= 3 duplicated helpers: $fires"
if [ "$fires" -ge 2 ]; then echo "outcome: rule fires"; else echo "outcome: not worth it"; fi
exit 0

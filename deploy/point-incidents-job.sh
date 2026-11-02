#!/usr/bin/env bash
# point-incidents-job.sh — print (or show) the one-line change that moves the Hermes incident job onto the repo's
# publisher. It never edits ~/.hermes by itself: a job's definition is the operator's, and this script's job is to
# make the change copy-pasteable and to say what it replaces.
#
# Usage: point-incidents-job.sh [--print]
#   --print   the exact command to run, and the job it belongs to (default)
#
# The job: "pii incidents to discord" (id cf97eea12eba), no_agent, every 15 minutes, script
# pii-incidents-discord.sh. Its file lives in ~/.hermes/scripts/ because a cron script path must be relative to it,
# so the wrapper stays there and only the LOGIC moves into the repository.
set -uo pipefail
JOB=${PII_INCIDENTS_JOB:-cf97eea12eba}
OLD=${PII_INCIDENTS_OLD:-pii-incidents-discord.sh}

cat <<EOF
The incident job ($JOB) runs ~/.hermes/scripts/$OLD, whose missing-record branch is a silent exit 0.
The publisher now lives in the repo. To point the job at it, run:

  cat > ~/.hermes/scripts/$OLD <<'WRAP'
#!/usr/bin/env bash
# The logic lives in the repository (scripts/autonomy/incidents-publish.sh) so it is reviewed and versioned.
exec bash /projects/pi-improved/scripts/autonomy/incidents-publish.sh "\$@"
WRAP
  chmod +x ~/.hermes/scripts/$OLD

Then the job keeps its id, its schedule and its delivery; only its behaviour changes:
  · a missing record now prints "the incident record is missing at …" and exits 2 (an error, not silence)
  · a record with no new rows stays silent (no_agent sends nothing on empty stdout)
  · new rows are printed once, tracked by _tmp/autonomy/.incidents-sent

Nothing here has been applied. The old file is not deleted by this script; move it to _trash once the wrapper works.
EOF

#!/usr/bin/env bash
# pii-podman.sh — the ONLY way pii touches containers. Referenced by the Skill document "pii-manages-podman".
#
# Why a wrapper exists rather than raw podman: a skill may reference a script but must never execute memory text, and the
# set of container operations pii is allowed to perform has to be enumerable. This file is that enumeration. Each verb is
# narrow on purpose; anything not listed is refused by name.
#
# Usage: bash scripts/pii-podman.sh <verb> [args]
#   ps [--all]                      list pii-owned containers (label io.pii=1)
#   run <image@sha256:…> <cmd…>     run detached-less, --rm, label io.pii=1, network none unless --net given
#   logs <container> [lines]        last N lines (default 80)
#   stop <container>                stop
#   rm <container>                  remove
#   inspect <container>             digest, status, labels
#   gc                              remove every label-matching container in "exited" state
#
# Guardrails enforced here, not by the caller:
#   · never --privileged, never --userns=host, never a bind of /var/run/docker.sock or /run/podman
#   · images must be pinned by digest (image@sha256:…) — a tag alone is refused
#   · every container carries the label io.pii=1 so `gc` and `ps` never touch anything else
#   · network is `none` unless --net <name> is passed explicitly
set -uo pipefail
PODMAN=${PODMAN_BIN:-/usr/bin/podman}
LABEL=io.pii=1

die() { echo "pii-podman: $*" >&2; exit 64; }
need() { [ -x "$PODMAN" ] || die "podman not found at $PODMAN"; }

verb=${1:-}; shift || true

case "$verb" in
  ps)
    need
    if [ "${1:-}" = "--all" ]; then
      exec "$PODMAN" ps -a --filter "label=$LABEL"
    fi
    exec "$PODMAN" ps --filter "label=$LABEL"
    ;;

  run)
    need
    img=${1:?usage: run <image@sha256:…> <cmd…>}; shift || true
    case "$img" in
      *@sha256:*) : ;;
      *) die "image must be pinned by digest (got '$img') — a tag can move under a running verdict" ;;
    esac
    net=none
    if [ "${1:-}" = "--net" ]; then net=${2:?--net needs a value}; shift 2; fi
    [ "$#" -gt 0 ] || die "run needs a command"
    workdir=${PII_WORKDIR:-$PWD}
    exec "$PODMAN" run --rm --label "$LABEL" --network "$net" \
      --workdir /work -v "$workdir:/work:Z" --security-opt no-new-privileges \
      "$img" "$@"
    ;;

  logs)
    need; c=${1:?usage: logs <container> [lines]}; n=${2:-80}
    exec "$PODMAN" logs --tail "$n" "$c"
    ;;

  stop)
    need; c=${1:?usage: stop <container>}
    exec "$PODMAN" stop "$c"
    ;;

  rm)
    need; c=${1:?usage: rm <container>}
    exec "$PODMAN" rm -f "$c"
    ;;

  inspect)
    need; c=${1:?usage: inspect <container>}
    exec "$PODMAN" inspect --format '{{.ImageName}} | {{.State.Status}} | {{index .Config.Labels "io.pii"}}' "$c"
    ;;

  gc)
    need
    ids=$("$PODMAN" ps -a --filter "label=$LABEL" --filter status=exited --format '{{.ID}}')
    [ -n "$ids" ] || { echo "pii-podman: nothing to collect"; exit 0; }
    echo "$ids" | xargs -r "$PODMAN" rm -f
    ;;

  *)
    echo "usage: pii-podman.sh {ps|run|logs|stop|rm|inspect|gc} …" >&2
    exit 64
    ;;
esac

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
#   images                          images present, with digests (fix triage starts here)
#   pull <img@sha256:…>             fetch a pinned image — fetching an artefact, NOT installing on the host
#   exec <container> <cmd…>         run a command inside a pii-labelled container only
#   restart <container>             restart a pii-labelled container
#   probe <img@sha256:…> <cmd…>     one-shot diagnostic container, network none, output only — the heal ladder's first act
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

  images)
    need; exec "$PODMAN" images --digests --format '{{.Repository}}:{{.Tag}} {{.Digest}}'
    ;;

  pull)
    need; img=${1:?usage: pull <image@sha256:…>}
    case "$img" in *@sha256:*) : ;; *) die "pull by digest only (got '$img')" ;; esac
    exec "$PODMAN" pull "$img"
    ;;

  exec)
    need; c=${1:?usage: exec <container> <cmd…>}; shift || true
    [ "$#" -gt 0 ] || die "exec needs a command"
    "$PODMAN" inspect --format '{{index .Config.Labels "io.pii"}}' "$c" 2>/dev/null | grep -qx 1 \
      || die "'$c' is not a pii container (label io.pii=1 missing) — refusing to exec into it"
    exec "$PODMAN" exec "$c" "$@"
    ;;

  restart)
    need; c=${1:?usage: restart <container>}
    "$PODMAN" inspect --format '{{index .Config.Labels "io.pii"}}' "$c" 2>/dev/null | grep -qx 1 \
      || die "'$c' is not a pii container — refusing to restart it"
    exec "$PODMAN" restart "$c"
    ;;

  probe)
    need; img=${1:?usage: probe <image@sha256:…> <cmd…>}; shift || true
    case "$img" in *@sha256:*) : ;; *) die "probe image must be pinned by digest" ;; esac
    [ "$#" -gt 0 ] || die "probe needs a command"
    exec "$PODMAN" run --rm --label "$LABEL" --network none --security-opt no-new-privileges "$img" "$@"
    ;;

  *)
    echo "usage: pii-podman.sh {ps|run|logs|stop|rm|inspect|gc|images|pull|exec|restart|probe} …" >&2
    exit 64
    ;;
esac

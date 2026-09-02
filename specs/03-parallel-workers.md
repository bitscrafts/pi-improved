# Spec: Parallel Workers

## Overview
Verify pidag `--concurrency 2` runs TWO parallel glm-5.3-flash:cloud workers on ollama.
**Project**: /projects/pi-improved
**Phase**: 0 - Toolchain validation (parallel)

## Requirements
### Functional
- F1: Two LLM worker jobs run concurrently and each produce an independent file.
- F2: Both must run in the SAME run (parallel fan-out, not sequential).

## Architecture
Two independent LLM nodes (worker-a, worker-b) with no dependency between them, so
pidag's `--concurrency 2` scheduler can dispatch them in parallel. Each writes its own file.

```
entry ──▶ worker-a (llm: write _tmp/par/a.txt = "A")
entry ──▶ worker-b (llm: write _tmp/par/b.txt = "B")
```

## TDD Contract
| Test Name | Input | Expected Output |
|-----------|-------|-----------------|
| parallel-a | worker-a | _tmp/par/a.txt contains "A" |
| parallel-b | worker-b | _tmp/par/b.txt contains "B" |

## Exit Criteria
- [ ] `test -f /projects/pi-improved/_tmp/par/a.txt`
- [ ] `test -f /projects/pi-improved/_tmp/par/b.txt`
- [ ] `cd /projects/pi-improved && test "$(cat _tmp/par/a.txt)" = "A"`
- [ ] `cd /projects/pi-improved && test "$(cat _tmp/par/b.txt)" = "B"`

## Guardrails
- Do NOT modify any file outside `_tmp/par/`.
- Each worker writes ONLY its own file.

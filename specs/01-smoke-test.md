# Spec: Smoke Test v2

## Overview
Verify the pidag + glm-5.3-flash:cloud workhorse pipeline works end-to-end.
**Project**: /projects/pi-improved
**Phase**: 0 - Toolchain validation

## Requirements
### Functional
- F1: Create a file `_tmp/smoke/hello.txt` containing the word "hello".
- F2: Then create `_tmp/smoke/second.txt` containing "done".

## Architecture
Single LLM node that writes the files.

## TDD Contract
| Test Name | Input | Expected Output |
|-----------|-------|-----------------|
| smoke | create hello.txt | file contains "hello" |

## Exit Criteria
- [ ] `test -f /projects/pi-improved/_tmp/smoke/hello.txt`
- [ ] `test -f /projects/pi-improved/_tmp/smoke/second.txt`

## Guardrails
- Do NOT modify any file outside `_tmp/smoke/`.

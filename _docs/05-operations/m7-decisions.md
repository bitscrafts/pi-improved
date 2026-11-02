# M7 — the dormant surface: six decisions, recorded

Each row is a *decision*, who made it, and what it obliges. A decision is only closed when the code or the
service matches the word here; "postponed" means nobody touches it and it is not a production dependency.

| # | surface | decision | who / when | what that obliges |
|---|---|---|---|---|
| 1 | `gemini` provider | **postponed** | operator, 2026-11-02 | no new code paths; it is not part of the production path and is not exercised |
| 2 | `matrix` channel | **postponed** | operator, 2026-11-02 | not configured, not polled |
| 3 | `telegram` channel | **near future** | operator, 2026-11-02 | kept present and unconfigured; becomes the second surface after Discord, and lands with a spec of its own |
| 4 | `webhook` channel | **kept — a valid insertion** | operator, 2026-11-02 | an external producer may post into pii through it |
| 5 | `gpiod` HAL | **postponed** | operator, 2026-11-02 | not part of the production path. It has no hardware to talk to: measured 2026-11-02, neither this container nor the lnx host has a `/dev/gpiochip*` |
| 6 | local `ollama.service` (on lnx) | **retire** | operator (AGENTS.md), reaffirmed 2026-11-02 | nothing new may be wired to a local ollama; disabling the host service is a deployment step for the operator, not an agent action |
| — | `mqtt` channel | **IN the production path** | operator, 2026-11-02 | must be exercised: its own poll interval and its own gate row (see `gates.sh`) |
| — | Discord | **the surface, now** | operator, 2026-11-02 | `pii` forum, grouped by topic; see `discord-topics.md` |

## What "postponed" is not

It is not "deleted": the code stays, compiled and linted by the gate, and the milestone does not depend on it.
It is also not "unowned" — each postponed row names who decided, so a later reader can tell a decision from an
oversight.

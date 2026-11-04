# Graph: `starter/crash`

**File**: `graphs/starter/crash.toml` · **nodes**: 1 · **edges**: 0 · **entry**: `boom` · **exit**: `—`

## What it is for

Deliberate crash trigger (F5.4 test seam), used only by scripts/phase3/crash.sh to exercise the daemon's panic hook.

## Shape

```mermaid
flowchart LR
  boom["boom<br/><i>debug_crash</i>"]
  boom:::entrynode
  classDef entrynode stroke-width:3px
  classDef exitnode stroke-width:3px,stroke-dasharray:4
```

## Nodes

| node | kind | key configuration |
|---|---|---|
| `boom` | `debug_crash` | — |

## Edges

_No explicit edges._

## Composition

- **Callable**: no — no `exit` node, so it cannot be inlined by `call`
- **Calls**: nothing
- **Called by**: nothing


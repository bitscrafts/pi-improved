# Graph: `starter/react`

**File**: `graphs/starter/react.toml` · **nodes**: 3 · **edges**: 3 · **entry**: `agent` · **exit**: `—`

## What it is for

_No header comment in the file._

## Shape

```mermaid
flowchart LR
  agent["agent<br/><i>agent</i>"]
  done["done<br/><i>transform</i>"]
  fail["fail<br/><i>transform</i>"]
  agent -->|ok| done
  agent -->|err| fail
  agent -->|budget| fail
  agent:::entrynode
  classDef entrynode stroke-width:3px
  classDef exitnode stroke-width:3px,stroke-dasharray:4
```

## Capabilities it holds

`["fs_read", "fs_write", "proc"]` — a node may hold only what its graph holds, and a `call` may grant only what its host grants. The ladder is enforced, not advisory.

## Limits

`max_turns=24`, `compact_at_tokens=24000`

## Nodes

| node | kind | key configuration |
|---|---|---|
| `agent` | `agent` | caps=`["fs_read", "fs_write", "proc"]` |
| `done` | `transform` | — |
| `fail` | `transform` | — |

## Edges

| from | port | to |
|---|---|---|
| `agent` | `ok` | `done` |
| `agent` | `err` | `fail` |
| `agent` | `budget` | `fail` |

## Composition

- **Callable**: no — no `exit` node, so it cannot be inlined by `call`
- **Calls**: nothing
- **Called by**: nothing


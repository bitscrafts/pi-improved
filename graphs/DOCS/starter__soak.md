# Graph: `starter/soak`

**File**: `graphs/starter/soak.toml` · **nodes**: 1 · **edges**: 0 · **entry**: `step` · **exit**: `—`

## What it is for

_No header comment in the file._

## Shape

```mermaid
flowchart LR
  step["step<br/><i>transform</i>"]
  step:::entrynode
  classDef entrynode stroke-width:3px
  classDef exitnode stroke-width:3px,stroke-dasharray:4
```

## Limits

`max_steps=100`, `max_wall_ms=30000`

## Nodes

| node | kind | key configuration |
|---|---|---|
| `step` | `transform` | — |

## Edges

_No explicit edges._

## Composition

- **Callable**: no — no `exit` node, so it cannot be inlined by `call`
- **Calls**: nothing
- **Called by**: nothing


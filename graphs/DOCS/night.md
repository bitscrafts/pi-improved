# Graph: `night`

**File**: `graphs/night.toml` · **nodes**: 4 · **edges**: 3 · **entry**: `tick` · **exit**: `report`

## What it is for

the night, as a composition of graphs pii already has. There is no `night_pick` and no `night_land` here. `queue` is called by pin (spec 255, extended by 343): the load inlines it and namespaces its ids (`sdd__select`, `sdd__record`), the run is ONE run with ONE journal, and a suspension inside the queue — a judge waiting on a gate, a timer — suspends the night at that node. The report is spec 12…

## Shape

```mermaid
flowchart LR
  tick["tick<br/><i>timer</i>"]
  sdd["sdd<br/><i>call</i>"]
  review["review<br/><i>transform</i>"]
  report["report<br/><i>post</i>"]
  tick -->|fired| sdd
  sdd -->|ok| review
  review -->|ok| report
  report:::exitnode
  tick:::entrynode
  classDef entrynode stroke-width:3px
  classDef exitnode stroke-width:3px,stroke-dasharray:4
```

## Capabilities it holds

`["spawn", "proc", "post", "fs_read", "fs_write"]` — a node may hold only what its graph holds, and a `call` may grant only what its host grants. The ladder is enforced, not advisory.

## Limits

`max_wall_ms=7200000`, `max_entries=12`

## Nodes

| node | kind | key configuration |
|---|---|---|
| `tick` | `timer` | — |
| `sdd` | `call` | workdir=`$.input.workdir` · graph=`queue` · caps=`["spawn", "proc", "fs_read", "fs_write"]` · pin=`1fafe548188f1300…` |
| `review` | `transform` | — |
| `report` | `post` | caps=`["post"]` |

## Edges

| from | port | to |
|---|---|---|
| `tick` | `fired` | `sdd` |
| `sdd` | `ok` | `review` |
| `review` | `ok` | `report` |

## Composition

- **Callable**: yes — it declares `exit = "report"`
- **Calls**: `queue` from node `sdd` (pinned 1fafe548188f…)
- **Called by**: nothing


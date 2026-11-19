# t9 performance

Goal from the design: CLI writes under 10 ms, small memory, scales to 100k+ entries.

## Results

Release build (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, stripped; binary 3.3 MB), Apple Silicon,
macOS 26. Median of 15 runs per command. **In-process** is t9's own time (`T9_TRACE=1`, from start of `main` to
exit). **Wall** includes starting the process; on the test machine starting any process costs about 6 ms
(`/bin/echo`), apparently because of endpoint-security software, and starting t9 costs about 10 ms.

| Command | Small: in-process | Small: wall | Large: in-process | Large: wall |
|---|---|---|---|---|
| `t9 log "…"` | 2.8 ms | 12.3 ms | 2.9 ms | 12.6 ms |
| `t9 add task "…"` | 3.2 ms | 13.2 ms | 7.5 ms | 17.5 ms |
| `t9 status ID active` | 3.1 ms | 14.4 ms | 3.9 ms | 14.4 ms |
| `t9 ls` | 2.0 ms | 12.0 ms | 5.4 ms | 15.2 ms |
| `t9 show ID` | 1.8 ms | 11.1 ms | 2.6 ms | 11.9 ms |
| `t9 context` | 2.6 ms | 11.8 ms | 16.8 ms | 29.9 ms |
| `t9 search` (2 words) | 2.9 ms | 12.1 ms | 28.7 ms | 38.8 ms |
| `t9 whereami` | 1.8 ms | 11.4 ms | 1.8 ms | 12.9 ms |

- Small: 50 items, 500 journal entries. Full reindex 0.65 s; peak RSS of `t9 context` 9.5 MB.
- Large: 2,000 items, 50,000 journal entries (26 MB vault). Full reindex 5.2 s; peak RSS 15.8 MB.
- The large search number is a worst case: the synthetic text uses 12 words, so both search words match nearly
  every one of the 52,000 documents. Real queries match far fewer.
- `add` in the large project lists the `items/` directory (2,000 files) to allocate the next id, which accounts
  for most of its time.

Automatic git commits add no measurable time to writes: `t9 log` took 11.0 ms wall with `T9_GIT=1` and with
`T9_GIT=0`. The commit runs in a detached process after t9 exits, and ten writes in a burst became two commits.

The Claude Code message hook (`t9 hook claude prompt`) on the large project, where about 400 decisions share one
small vocabulary (a worst case), takes 3.1 ms in-process (16 ms wall) when nothing matches, and 5.5–5.7 ms
(about 19 ms wall) when it prints matches.

## How to reproduce

```sh
cargo build --release
python3 scripts/bench.py            # --runs N, --t9 PATH
```

`scripts/gen_vault.py` builds the synthetic vaults; `scripts/bench.py` builds both sizes in a temp directory,
reindexes, and prints the table. `T9_TRACE=1 t9 <command>` prints per-phase timings (open, sync check, sync
apply, items query, total) for any single command.

## What mattered

Measured problems found while building, and their fixes:

| Problem | Before | After | Fix |
|---|---|---|---|
| FTS rows deleted by an `UNINDEXED` text column (full table scan per delete) | reindex 206 s | 4.3 s | External-content FTS5 tables keyed by an integer rowid, kept in sync by triggers |
| Every read listed and `stat`ed every vault file (~30 µs per `stat` here) | sync 79 ms | ~1 ms | Directory-mtime fast path, own writes record the new directory state, recent journals checked directly, full check every 5 min |
| Snippets computed for every match before `UNION … ORDER BY … LIMIT` | search 520 ms | 29 ms | One query per table using FTS5's internal `ORDER BY rank LIMIT`, merged in Rust |
| Newest-first and per-kind entry queries sorting 50k rows | context 63 ms | 17 ms | Indexes `(project, at, id)` and `(project, kind, at, id)`; counts with `count(*)` instead of loading rows |
| Concurrent writers failing with "database is locked" | failures under 24 parallel writers | none | `BEGIN IMMEDIATE` for write transactions, opened only after a read-only check |

## Where the remaining time goes

- Process start (~10 ms wall) dominates every small command. A long-lived `t9 serve` (planned) answers in well
  under a millisecond for the TUI and any agent integration that prefers HTTP.
- `context` on large projects loads all items (JSON-decoding 2,000 front-matter rows, ~4 ms) and checks recent
  activity for each active item. Both can be narrowed if projects grow far beyond this size.

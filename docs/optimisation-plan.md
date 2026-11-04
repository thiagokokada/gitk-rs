# Optimisation plan

Status: analysis complete; runtime optimisations have not started. Resume this work
in a future session. Keep each validated change in a separate commit.

## Baseline: real nixpkgs history

On 2026-11-04, both applications were driven through **100,000 and 200,000 loaded
commits**, twice each, on the same nixpkgs checkout. Counts were verified in logs
and screenshots. The `nix` filter returned **30,026 matches from 200,000 commits**
in both applications; clearing it restored the history.

| Loaded commits | Rust application RSS | Go application RSS | Git child RSS, either app |
| --- | --- | --- | --- |
| 100,000 | 335-336 MiB | 561-564 MiB | 326-327 MiB |
| 200,000 | 562-563 MiB | 1,171-1,216 MiB | About 438 MiB |

Application RSS excludes Git. Neither application's process showed swap at these
checkpoints. Git's RSS includes substantial clean file-backed mappings; it must
not be described as application heap. At 200,000, Rust's Git child had about
406 MiB private clean pages and 30 MiB anonymous memory.

The broad-filter observation consumed 0.15-0.16 CPU seconds in Rust and
0.64-0.66 in Go. These include UI/diff work over a fixed observation interval,
not input-to-paint latency. Screenshots showed the completed filter in both apps.
Rust's five-second idle samples used 0.4-0.6% of one core; Go's rounded to zero
at the sampling resolution.

The balanced run accumulated about 5.47 versus 3.25 application CPU seconds by
100,000 commits, and 10.81 versus 9.07 by 200,000, for Rust versus Go. This suggests
investigating Rust's loading CPU cost despite its lower retained memory. It does
not isolate parsing, graph building, rendering, or highlighting.

### Scope and limitations

- Release packages: Rust revision `72a1803`, Go revision `fc472ac`.
- nixpkgs HEAD: `d72d63e4ca69cbc3141157034a1302eb84894a12`.
- Linux/NixOS, Ryzen 5 7600X, 32 GiB RAM, X11 via XWayland.
- Same 1920x1332 window, dark mode, syntax enabled, 200 graph lanes,
  10,000-commit batches, and automatic refresh disabled.
- Rust caps batches at 10,000 internally. A large `--limit` alone does not prove
  that 100,000 commits were loaded: browsing demand must advance the stream.
- Fresh preferences/cache per launch; OS caches were not flushed. Both application
  orders were exercised. There is no persistent history index to warm.
- The balanced run waited for complete 10,000-commit boundaries before each End
  key. The initial driver also reacted to partial Rust snapshots. Its extra
  selections make its navigation timing unsuitable for direct comparison.
- Wall times contain driver delays and the pause for measurements at 100,000;
  do not present them as parser throughput or a startup-speed comparison.
- This is a small observational sample, not an isolated statistical benchmark.
  Native Wayland, macOS, million-commit browsing, precise frame latency, and
  pathological large diffs remain unmeasured.

Exact observations and provenance are in
[optimisation-baseline.json](optimisation-baseline.json). Raw local logs,
screenshots, and 100 ms process samples were saved under
`/tmp/gitk-rs-large-history-results`; those temporary files may be cleaned up.
The temporary diagnostic drivers were not added to the repository.

## Findings that determine priority

1. **Retained graph geometry is the largest identified Rust allocation.**
   Accounting over the same 200,000 commits found about 293 MiB of edge-vector
   capacity for 229 MiB of live edge payload: roughly **64 MiB of spare capacity**.
   Parent-ID vectors reserve about 18.3 MiB for 6.5 MiB of live elements.
   This accounts for payload capacities, not allocator overhead or a heap profile.
   The 200-lane cap is frequently reached: 115,822 rows carry the overflow flag.
   Preserve that behavior when changing representations.
2. **Graph construction does repeated work.**
   [GraphBuilder::push](../src/graph.rs) clones the lane frontier, including owned
   target-ID strings, for every commit. It then repeatedly performs linear target
   lookups while walking that frontier. A wide graph can make this quadratic in
   lane count. This is a code-derived candidate; no CPU profile attributes a
   percentage of the observed loading time to it yet.
3. **Large-hunk highlighting repeats prefix parsing.**
   [DiffDocument::highlight](../src/diff.rs) starts at the hunk boundary on each
   page request. Scrolling deep into one large hunk repeatedly parses its prefix.
   This stress run exercised ordinary commit diffs, not that worst case.
4. **Filtering and idle polling are secondary.**
   [Commit::searchable_text](../src/git.rs) allocates and lowercases combined text
   on every scan, unlike Go's cached text. Nevertheless, the measured Rust filter
   was already inexpensive at 200,000 commits. UI mailbox timers poll every 16 ms,
   Git cancellation workers every 5 ms, and workspace reconciliation every 100 ms.
   These are candidates for later work rather than reasons to redesign history.

## Increment 1: reduce retained graph allocation

- [ ] Add focused, opt-in Rust allocation accounting for real history and a small
  deterministic wide-graph fixture; keep large runs out of normal CI.
- [ ] Reduce unused edge-vector capacity without changing the logical graph.
  Compare exact reservation and compact final storage. Measure the CPU cost of
  shrinking or copying each row before choosing a representation.
- [ ] Address spare parent-vector capacity if measurements justify a separate
  small change. Preserve typed IDs and SHA-1/SHA-256 behavior.
- [ ] Repeat the matched 100,000/200,000-commit workload and report application,
  Git child, and anonymous/private memory separately.

Acceptance: node, edge, color, merge, lane-hole, overflow, and lane-cap behavior
are unchanged; selection and deep scrolling still work. Show measured memory
savings and any loading-CPU tradeoff. Do not claim the theoretical 64 MiB as an
achieved RSS reduction before measuring it.

## Increment 2: reduce graph construction work

- [ ] Profile the unchanged loading path on wide real history before changing it.
  Separate Git reading/parsing, graph preparation, and UI application where useful.
- [ ] Avoid cloning every lane's owned target string for every row.
- [ ] Replace repeated frontier scans if profiling confirms their cost. Keep any
  lookup storage bounded by the configured lane count and preserve stable lanes.
- [ ] Test chains, branch/merge patterns, octopus merges, sparse lane holes,
  overflow at several caps, and color continuity. Compare complete logical rows
  against the previous implementation using deterministic fixtures.
- [ ] Repeat the baseline and record CPU, elapsed workload time, and memory.

Acceptance: equivalent graph output and navigation behavior, lower measured
loading cost, no whole-history lookup index, and no new retained duplication.

## Increment 3: bound repeated large-hunk highlighting work

- [ ] Add an opt-in large single-hunk fixture and measure successive page requests,
  deep jumps, reverse scrolling, and rapid cancellation.
- [ ] Retain parser state or sparse checkpoints within a byte budget per active
  diff. Preserve separate old/new parser states for removed and added code.
- [ ] Invalidate on document/selection and theme changes; reject stale results.
- [ ] Add unit and worker tests for multiline comments/strings, Unicode, hunk/file
  boundaries, theme switches, cancellation, and checkpoint eviction.

Acceptance: paged colors match parsing from the hunk start, deep pages do less
repeated work, and cache bytes remain bounded. Retain full language coverage.

## Increment 4: optional work after profiling

- [ ] If filtering becomes material, reuse scratch storage or use an allocation-
  avoiding fast path with equivalent Unicode and cross-field matching semantics.
  Avoid permanently duplicating every commit's message as cached search text.
- [ ] Replace idle mailbox polling with notifications, and make cancellation
  wakeups event driven where practical. Preserve prompt child termination and
  reaping; do not simply lengthen polling intervals to improve idle measurements.
- [ ] Test missed/coalesced notifications, stale generations, shutdown, watcher
  failure, and manual reload. Compare idle CPU with watching on and off.

Acceptance: measured benefit, unchanged workflows, bounded pending work, and no
stalled shutdown or missed refresh. Defer these changes if profiling shows little
practical value.

## Validation and comparison procedure

Use the pinned Nix toolchain and current release builds. Start both applications
against the same fixed nixpkgs HEAD with:

```sh
BINARY --limit 10000 --graph-cols 200 --mode dark --nowatch --verbose REPOSITORY
```

Advance to each loaded-history boundary, verifying actual counts rather than
assuming the CLI limit is a total. At 100,000 and 200,000, wait for background
work to settle, sample application and child processes separately, then exercise
navigation and a broad filter. Capture own-window screenshots. Do not let a rare
query silently load the rest of history and invalidate the count comparison.
Repeat with identical settings and record cache conditions and any driver delays.
Use focused fixtures for highlighting; history count does not stress a large hunk.
Do not recreate the removed Python release probe as part of runtime optimisation.

For each implementation increment, run relevant unit, real-Git worker, and
headless GPUI tests, plus the standard checks:

```sh
nix develop --command cargo fmt --all -- --check
nix develop --command cargo clippy --all-targets -- -D warnings
nix develop --command cargo test --all-targets --locked
nix build
nix flake check
```

Smoke-test native X11 and Wayland for UI changes, and use macOS CI for portability.
Report unavailable checks explicitly. Keep one Git stream and demand-driven
loading; do not introduce SQLite, a persistent history cache, repeated deep
`--skip` scans, or a lower graph cap to manufacture a benchmark improvement.

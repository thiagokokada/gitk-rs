# GPUI Fast comparison

The default backend remains upstream GPUI through GPUI Kit 0.9.0. The optional
`gpui-fast` feature selects GPUI Fast across Kit's core, platform, components,
and assets without changing application imports. Cargo.lock pins Fast to 0.1.13.
The lockfile also contains optional backends that a default build does not use.

```sh
nix develop --command cargo build --release --locked --features gpui-fast
nix develop --command cargo test --all-targets --locked --features gpui-fast
nix develop --command cargo clippy --all-targets --locked --features gpui-fast -- -D warnings
```

## Reproducing the UI probe

Run each backend in a separate process, alternating their order across runs:

```sh
nix develop --command cargo test --release --bin gitk-rs --locked \
  profile_ui_scrolling -- --ignored --nocapture
nix develop --command cargo test --release --bin gitk-rs --locked \
  --features gpui-fast profile_ui_scrolling -- --ignored --nocapture
```

The ignored probe uses the real application views and worker threads with a
deterministic temporary Git fixture: 5,000 linear commits and an untracked Rust
file with 2,000 lines. It loads history to EOF and selects the local diff, then
scrolls history and diff separately in a 1280x850 logical-pixel headless window
at scale 2, with graph lane cap 256 and accessibility disabled. Each scenario
uses 40-pixel wheel steps, reversing every 16 steps, discards 32 warmup samples,
and reports median and p95 over 240 samples.

Each sample measures wall time for input dispatch, runnable test tasks and one
explicit draw. Worker page loading, highlighting waits, fixture creation and
additional settling draws are outside the samples. The test uses GPUI's test
text system and no GPU renderer: these are UI preparation measurements, not
native text shaping, GPU completion, display latency or FPS measurements.
Settling draws deliberately avoid `refresh()` and Kit's `render_frame()`, which
force a full refresh and would bypass retained views.

Fast's [retained-mode documentation](https://github.com/longbridge/gpui-fast/blob/v0.1.13/docs/retained-mode.md)
and [scroll-layer documentation](https://github.com/longbridge/gpui-fast/blob/v0.1.13/docs/scroll-layers.md)
describe its reuse and invalidation rules. Changes outside tracked entity state
must notify their view. The application already virtualizes both lists and
keeps Git, history and highlighting on workers; changing the UI backend does
not reduce retained history storage or accelerate those workers directly.

Native Wayland, X11 and macOS testing remains separate from this probe.

## Virtual versus eager history

The second ignored probe compares the normal `uniform_list` with a test-only
scrolling `div` that constructs every commit row. Both use the application's
same row renderer, graph canvases, chrome, scroll handle and complete display
snapshot. This intentionally removes history paging from the comparison.
The eager path is compiled only for tests; normal builds always virtualize.
Worker snapshots remain bounded at 512 rows. During fixture preparation only,
the probe collects their shared entries into one display snapshot for both
strategies, outside all measurements.

For each backend, repeat with `GITK_PROFILE_LIST=virtual` and `eager`, and
`GITK_PROFILE_ROWS=100`, `1000` and `5000`:

```sh
nix develop --command env GITK_PROFILE_LIST=eager GITK_PROFILE_ROWS=5000 \
  cargo test --release --bin gitk-rs --locked --features gpui-fast \
  profile_commit_list -- --ignored --nocapture
```

It reports the first draw after explicitly notifying the history view, then
discards 32 warmup steps and measures 96 samples using the same wheel steps
as the scrolling probe.
Both probes assert that wheel input actually moved the list. This comparison
uses a linear graph and a short back-and-forth scroll, not a deep-scroll or
million-commit benchmark. Eager rendering's first draw includes constructing
all rows; steady scrolling may reuse already constructed content under Fast.

## Results, 2026-11-09

Apple M2 Pro, 32 GiB RAM, macOS 27.0.1, pinned Nix Rust 1.99.0, release builds.
Both native binaries report `0.1.0-eb832a8`; the final probe's source hash is
recorded with the [raw measurements](measurements/gpui-fast-2026-11-09.json).
Three fresh test processes per scenario and backend were run after compilation
finished. Backend order was upstream/Fast, Fast/upstream, upstream/Fast.
These tables show the median of the three per-run medians; p95 columns are
the median of the three per-run p95 values, not a pooled percentile.

| UI preparation sample | Upstream median | Fast median | Upstream p95 | Fast p95 |
| --- | ---: | ---: | ---: | ---: |
| History scrolling, local diff visible | 1.734 ms | 1.182 ms | 2.103 ms | 1.234 ms |
| Highlighted diff scrolling | 1.706 ms | 0.932 ms | 1.892 ms | 0.947 ms |

Fast reduced these sample medians by 31.8% and 45.4%. Its counters confirmed
view reuse, but reported zero scroll-layer composites in both scenarios.
Counters are read before and after each scenario without resetting them:
resetting would enable Fast's additional measurement timers. These headless
results do not establish native GPU scrolling or battery improvements.

| Commit rows | Upstream virtual | Upstream eager | Fast virtual | Fast eager |
| --- | ---: | ---: | ---: | ---: |
| 100 | 1.427 ms | 5.336 ms | 0.971 ms | 2.802 ms |
| 1,000 | 1.435 ms | 53.282 ms | 0.964 ms | 26.215 ms |
| 5,000 | 1.422 ms | 303.475 ms | 0.952 ms | 154.055 ms |

The eager list was slower even at 100 rows. At 5,000 rows, its Fast scrolling
sample was about 162 times the virtual list's. The first notified draw also
cost more: 249.257 ms eager versus 1.209 ms virtual on Fast, and 381.669 ms
versus 1.989 ms on upstream. These are display-construction costs after loading
the fixture, not application startup. Keep history virtualization enabled.

### Native macOS startup and memory

Each native binary opened the same 5,000-commit fixture and 2,000-line local
Rust diff for five seconds with `--timings --nowatch --no-limit --mode=dark
--graph-cols=256 --nosyntax=false`, then was deliberately terminated with
SIGTERM. Three launches per backend used the same alternating order and saved
font/layout settings. All six reached all four startup paint markers.
Startup markers measure from `main()` to CPU callbacks, not GPU completion or
screen presentation. The native test did not inject scrolling input.

| Native measurement, median of three launches | Upstream | Fast |
| --- | ---: | ---: |
| First window paint callback | 497.696 ms | 533.393 ms |
| First history/selection paint callback | 576.588 ms | 608.845 ms |
| Maximum sampled application RSS | 157.547 MiB | 159.484 MiB |
| Maximum sampled live-child RSS, summed separately | 7.391 MiB | 0.125 MiB |

There was no clear startup improvement: first-history markers ranged from
576.566 to 1,050.647 ms upstream and 574.312 to 912.259 ms on Fast. Fast's
median application RSS was about 1.94 MiB higher in these short launches.
RSS was sampled with `ps -axo pid=,ppid=,rss=` and a 50 ms sleep between calls;
process-inspection overhead made actual intervals longer. These are observed
maxima, not peak memory or macOS physical footprint. Child processes were
sampled independently from the application and can exit between samples;
the child figures do not establish a reduction in Git memory use.

### Validation and remaining coverage

- Formatting and Clippy with warnings denied passed for the relevant targets.
- Both backends passed the ordinary Rust suite: 168 tests, with five opt-in
  tests ignored. All 42 opt-in release probe processes in the comparison passed.
- `nix build` and `nix flake check` passed on aarch64-darwin. The latter explicitly
  omitted aarch64-linux and x86_64-linux on this host.
- An initial Fast suite run failed the transient branch-success notice assertion
  in `branch_protection_and_selected_ancestor_survive_switch`; its isolated rerun
  and subsequent complete suites passed. No branch protection was disabled.
- Native Wayland/X11 smoke tests, native scrolling/GPU timing, accessibility-on
  performance, and million-commit comparisons were not run.

Keep Fast opt-in: UI preparation improved in these fixtures, while native startup
and memory did not show a corresponding benefit. Retain virtual history and
diff lists on either backend.

## Read-only repository probe

`profile_repository_scrolling` opens `GITK_PROFILE_REPOSITORY` with the real
history, workspace and diff workers. It disables watching and streams history
to EOF by default. `GITK_PROFILE_COMMITS` optionally sets a prefetch target;
this probe needs more than 10,128 loaded commits for its deeper viewport.
It never checks out a branch or writes files in the measured repository.
Its preferences and view state are isolated by the test harness.

Set `GITK_PROFILE_DIFF_COMMIT` to a commit in the first 512 rows with more
than 64 diff lines. The probe selects it, then measures history scrolling
near the top, history around row 10,000, and highlighted diff scrolling.
Each scenario uses the earlier probe's 32 warmup and 240 measured wheel
samples, headless text system, window size, graph cap and settling rules.
Loading and page/highlighting waits are outside scrolling samples. Initial
loading may take up to 180 seconds; its reported time includes test setup
and UI updates while history streams, and is not Git-only throughput.

```sh
nix develop --command env \
  GITK_PROFILE_REPOSITORY="$HOME/Projects/nixpkgs" \
  GITK_PROFILE_DIFF_COMMIT=eb82e507d5e2ed0a71bb6f2f7c37d3ba547399e1 \
  cargo test --release --bin gitk-rs --locked --features gpui-fast \
  profile_repository_scrolling -- --ignored --nocapture
```

Omit `--features gpui-fast` for the upstream comparison. Keep the same repository
HEAD and loaded count, and alternate backend order across runs. The command's
example diff commit belongs to the nixpkgs HEAD recorded with the results;
another HEAD may require choosing another recent commit.

See the [full-history nixpkgs comparison](gpui-fast-nixpkgs.md) for results on
1,114,966 commits.

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

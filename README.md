# gitk-rs

A Rust/GPUI port of gitk-go for Linux/NixOS, designed for streaming Git history browsing
with virtualized rendering. Loaded rows stay in memory like gitk-go; no persistent
history database/cache is planned. V1 prioritizes functionality, with performance
aspirations guiding design rather than blocking delivery. See [PLAN.md](PLAN.md) for milestones and acceptance
criteria and [AGENTS.md](AGENTS.md) for contributor instructions.

**Status:** the Linux viewer includes streamed history, graph/ref labels, filtering,
selectable highlighted diffs, local staged/unstaged/untracked changes, protected
branch switching, repository opening, automatic refresh, themes and saved font
preferences. Release acceptance is tracked in PLAN.md.

## Development

Nix with flakes enabled supplies Rust and native libraries. The flake pins nixpkgs;
the shell deliberately does not depend on the application package, so it can be
used without first building the application:

```sh
nix develop
cargo fmt --all
cargo test --all-targets --locked
cargo clippy --all-targets -- -D warnings
cargo run -- --mode dark /path/to/repository
```

Cargo.lock and flake.lock pin the dependency graph and Nix environment. Verify
packaging with:

```sh
nix build
nix flake check
./result/bin/gitk-rs --help
./result/bin/gitk-rs /path/to/repository
```

The viewer accepts --mode dark|light, --timings, --limit N, --graph-cols N,
--text-graph, --nosyntax, --nowatch, --verbose, --list-languages, --licenses,
--help and --version. A path beginning
with a dash can follow --. Repository paths may contain spaces or non-UTF8 bytes.
The GUI uses the published [GPUI 0.2.2 API](https://docs.rs/gpui/0.2.2/gpui/).
Linux enables Wayland and X11; macOS dependency configuration is preparatory and
has not been validated. A desktop session and a working GPU driver are required
for GUI smoke tests. The package supplies loaders, not a machine-specific driver.

Use arrows or j/k/n/p to select commits, Home/End for first/last loaded, Ctrl+Page
Up/Down to scroll history, / to focus the filter, Escape to leave it and F5 to
reload. Space/Backspace/b page the diff; u/d scroll 18 lines. Drag to select text,
Shift-click to extend, double-click for a word, and use Ctrl+A in the diff for all
lines. Ctrl+C copies; Ctrl+Shift+C removes diff markers from code lines. Copying
reads the temporary document, including lines outside the current viewport.

The [213 bundled syntaxes](docs/LANGUAGES.md) work offline, with filename and shebang
detection. Themes keep syntax foregrounds over added/removed backgrounds. Large
diffs use an unlinked temporary spool, capped at 256 MiB or one million lines;
individual lines display up to 16 KiB and clipboard selections up to 16 MiB. Limits
are reported in the viewer. Hunk gaps reset parsing state, so partial multiline
constructs can be imperfectly colored. Grammar/theme acknowledgements ship in
assets/licenses and the Nix package's share/doc/gitk-rs directory.
Ctrl+O opens a repository using the desktop file picker. Ctrl+Shift+O opens
a path-entry dialog, also used as a fallback when the picker is unavailable. Ctrl+B opens local
branches; arrows and Enter select one. Git errors remain visible without discarding
changes. F1 opens shortcut help. The Menu button exposes these actions, copy commit
reference, font preferences, automatic reload, theme and About. Right-click a commit
or file for copying; the diff menu offers both selection-copy modes.

Ctrl+, opens font preferences; Ctrl+Enter applies and saves. Preferences live at
$XDG_CONFIG_HOME/gitk-rs/preferences.json (normally ~/.config/gitk-rs). Font families
refer to installed system fonts. Alt+T switches theme; Alt+R toggles automatic
reload. The filter keeps the original Emacs controls: Ctrl+A/E go to start/end,
Ctrl+B/F move a character, Ctrl+W deletes a word, Ctrl+U/K delete to start/end and
Ctrl+D deletes forward. Escape returns focus to history.

Automatic refresh uses a bounded set of nonrecursive watches plus a five-second
background status reconciliation. Watch failures leave periodic/manual refresh
available; --nowatch disables automatic refresh. No history data persists.
The small [xattr compatibility patch](vendor/README.md) is required by GPUI's
transitive archive dependency on current libc releases.

--timings reports window creation and first CPU paint callback timestamps to stderr
and a bounded recent-sample CPU render-to-paint p95 on graceful exit. Its writer
runs off the UI thread. These measurements exclude process-loader time and do not
measure compositor presentation or GPU execution. HistoryPainted and
SelectionPainted are first CPU paint callbacks with those views populated.

For a small read-only backend smoke check without opening a window:

```sh
nix develop --command cargo run --locked --example history_probe -- /path/to/repo 1000
```

## Environment limitations observed

In the restricted execution sandbox, connecting to the Nix daemon socket returns
Operation not permitted despite its world-writable mode. DNS lookup of crates.io
and GitHub also fails. These are observed execution-environment restrictions;
they do not imply a problem with Nix in the host user's terminal.

An approved execution exception now permits running Nix outside that sandbox.
The pinned development shell, dependency downloads, formatting, lint/tests,
packaged build and flake checks now succeed through that exception. This does not
change host Nix permissions. See PLAN.md for validation details and pending work.

The initial pin inherited from gitk-go used an older glibc than this machine's
graphics driver. Updating the pin to the host nixpkgs revision resolved Vulkan
initialization on both backends. If a future NixOS upgrade causes GPU startup
failures, Vulkan loader diagnostics can distinguish ABI incompatibility from
missing devices or drivers; do not work around it by forcing a vendor-specific
driver in the package.

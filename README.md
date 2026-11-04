# gitk-rs

A lightweight Git history explorer for Linux, written in Rust with
[GPUI](https://gpui.rs/).

![Screenshot](screenshot.png)

## Features

- Commit history with a branch graph, author and date columns
- Background loading and filtering to browse large repositories
- Syntax-highlighted diffs with support for more than 200 languages
- File list to jump between changes, with text selection and copying
- Staged, unstaged and untracked changes
- Branch switching, automatic refresh and keyboard navigation
- Light and dark themes with configurable fonts

## Usage

Requires Linux and Nix with flakes enabled. From this checkout:

```sh
# Browse the current repository
nix run .

# Browse another repository
nix run . -- ~/Projects/nixpkgs

# Use the light theme
nix run . -- --mode light ~/Projects/nixpkgs
```

You can also build and run the executable directly:

```sh
nix build
./result/bin/gitk-rs /path/to/repository
```

Use the arrow keys or j/k to select commits, / to filter, and Space/Backspace
to page through a diff. Ctrl+O opens a repository, Ctrl+B switches branches,
and F5 reloads. Press F1 for all shortcuts.

For command-line options or the supported language list:

```sh
nix run . -- --help
nix run . -- --list-languages
```

## Development

```sh
nix develop
cargo run -- /path/to/repository
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets --locked
```

Tests include unit tests, Git integration tests and GPUI interaction tests that
run without a display. Run `nix flake check` to check formatting and packaging.
See [AGENTS.md](AGENTS.md) for contributor guidelines.

## License

[MIT](LICENSE). Bundled grammar, theme and icon notices are in
[assets/licenses](assets/licenses/README.md); `gitk-rs --licenses` prints the
embedded grammar and theme acknowledgements.

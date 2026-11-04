# gitk-rs

![Icon](resources/icons/gitk-rs.svg)

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

Requires Linux, Git and a Vulkan-capable graphics driver.

```sh
# Browse the current repository
./gitk-rs

# Browse another repository
./gitk-rs ~/Projects/nixpkgs

# Use the light theme
./gitk-rs --mode light ~/Projects/nixpkgs
```

You can also build and run the executable directly:

```sh
./gitk-rs /path/to/repository
```

Use the arrow keys or j/k to select commits, / to filter, and Space/Backspace
to page through a diff. Ctrl+O opens a repository, Ctrl+B switches branches,
and F5 reloads. Press F1 for all shortcuts.

For command-line options or the supported language list:

```sh
./gitk-rs --help
./gitk-rs --list-languages
```

## Development

### Building

Install a current stable Rust toolchain with [rustup](https://rustup.rs/).
On Ubuntu 24.04, install the native dependencies:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config cmake clang libclang-dev git \
  libfontconfig1-dev libfreetype6-dev libssl-dev zlib1g-dev libwayland-dev \
  libxcb1-dev libxkbcommon-x11-dev libx11-dev libxcursor-dev libxi-dev \
  libxrandr-dev libvulkan-dev fonts-dejavu-core
```

Then build and run:

```sh
cargo build --release --locked
./target/release/gitk-rs /path/to/repository
```

#### Nix

Assuming that you have [Flakes](https://wiki.nixos.org/wiki/Flakes) enabled:

```sh
nix build
./result/bin/gitk-rs /path/to/repository
```

### Checks

With the dependencies above installed, or after entering `nix develop`:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

Run `nix flake check` to check formatting and packaging.

## License

[MIT](LICENSE).

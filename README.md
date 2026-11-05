# gitk-rs

![Icon](resources/icons/gitk-rs.svg)

A lightweight Git history explorer for Linux and macOS, written in Rust with
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

Requires Git and either Linux with a Vulkan-capable graphics driver or macOS
with Metal support.

```sh
# Browse the current repository
./gitk-rs

# Browse another repository
./gitk-rs ~/Projects/nixpkgs

# Use the light theme
./gitk-rs --mode light ~/Projects/nixpkgs
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

On macOS, install the Xcode Command Line Tools and build tools with
[Homebrew](https://brew.sh/):

```sh
xcode-select --install
brew install cmake pkg-config
```

Then build and run on either platform:

```sh
cargo build --release --locked
./target/release/gitk-rs /path/to/repository
```

On macOS, builds compile Metal shaders at startup by default. To compile them
at build time, install and open full Xcode to finish setup, then run these commands
outside `nix develop`:

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
/usr/bin/xcodebuild -downloadComponent MetalToolchain
/usr/bin/xcrun -sdk macosx --find metal
/usr/bin/xcrun -sdk macosx --find metallib
cargo build --release --locked --no-default-features
./target/release/gitk-rs /path/to/repository
```

The Command Line Tools alone may lack the Metal compiler. See Apple's
[additional Xcode components](https://developer.apple.com/documentation/xcode/downloading-and-installing-additional-xcode-components)
instructions for installing the toolchain.

#### Nix

Assuming that you have [Flakes](https://wiki.nixos.org/wiki/Flakes) enabled:

```sh
nix build
./result/bin/gitk-rs /path/to/repository
```

Nix uses runtime shaders by default. Its SDK does not provide the Metal compiler,
and sandboxed builds cannot automatically use Xcode from `/Applications`.
The package accepts `metalToolchain` as an override: a Nix store package providing
`bin/xcrun` with `metal`, `metallib`, and their dependencies. Supplying it enables
build-time compilation, but a packaged toolchain is not yet included here.

### Checks

With the dependencies above installed, or after entering `nix develop`:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
```

Run `nix flake check` to check formatting, packaging and the executable.

## License

[MIT](LICENSE).

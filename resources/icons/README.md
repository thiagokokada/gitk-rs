# Application icon

`gitk-rs.svg` is adapted from gitk-go by Thiago Kenji Okada and covered by
the root [MIT license](../../LICENSE). `gitk-rs.png` is a rasterized copy of
that same artwork, embedded for the macOS Dock and Cmd+Tab application icon.

After editing the SVG, regenerate the PNG from the repository root:

```sh
nix run --inputs-from . nixpkgs#resvg -- resources/icons/gitk-rs.svg resources/icons/gitk-rs.png --width 1024
```

The macOS integration runs in GPUI's startup callback on the main thread.
The `setApplicationIconImage` binding requires an unsafe call because its
image argument may not accept null. `src/macos.rs` permits unsafe code only
in the small setter function, which always supplies a decoded image.
Unsafe code remains denied by default.

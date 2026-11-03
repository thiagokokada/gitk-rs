# Compatibility patches

`xattr/` contains xattr 0.2.3 from crates.io, retaining its original MIT and
Apache-2.0 licenses. GPUI 0.2.2 depends on zed-async-tar 0.5.0-zed, which enables
this old xattr release through its default features.

The only source change is in src/sys/mod.rs: use libc::ENODATA on Linux/Android
instead of libc::ENOATTR, an obsolete alias removed from newer libc versions.
Other platforms retain their original constant. No runtime behavior changes:
Linux reports ENODATA when an attribute is absent.

Remove this patch when the GPUI dependency chain accepts a current xattr release.
Do not edit the Cargo registry cache to apply it: Cargo and Nix builds must use
the same checked-in source through the root manifest's patch table.

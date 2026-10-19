# Implementation decisions

## Stack

- **Rust 2021, stable toolchain:** mature compiler support without requiring nightly features.
- **`fuser` 0.16 with the `abi-7-31` feature:** implements the Linux FUSE protocol in-process and targets the FUSE 3.10-era ABI while avoiding a runtime dependency on `libfuse`.
- **`libc` 0.2:** supplies Linux error numbers, open flags, and filesystem constants used by FUSE callbacks.
- **`chrono` 0.4 with only `clock` and `std`:** computes local calendar-day boundaries for `today` and `yesterday`; rolling windows use `SystemTime` durations.
- **`clap` 4 with derive support:** provides a small, validated CLI for source and mount point selection.
- **Nix flakes:** provide the Rust build environment plus FUSE 3 and test utilities reproducibly.
- The packaged launcher appends Nix's FUSE 3 tools to `PATH` instead of prepending them, so a host-provided privileged `fusermount3` wrapper remains preferred. The Nix binary is retained as a fallback on systems where unprivileged mounting does not require a privileged helper.

## Filesystem architecture

- The filesystem is a live projection with no index or cache. Every lookup and directory listing consults the source directory, so direct source changes appear immediately.
- Virtual inode numbers are stable hashes of a view plus the source-relative path. Hash collisions are detected while the process is running and resolved deterministically.
- Category views expose top-level regular files only. The `dirs` view maps top-level source directories and all of their descendants one-to-one.
- File descriptors are opened read-only with `O_NOFOLLOW`; mutation callbacks operate on validated parent/name pairs relative to the canonical source root.
- Category roots reject creation, writes, and rename. Deletion is allowed and affects the backing entry. The source root itself is never represented as a deletable node.
- Renames inside the mounted filesystem are not supported. Moving out relies on the normal cross-filesystem `EXDEV` path: read/copy through FUSE, then `unlink` or `rmdir` through FUSE only after the destination copy succeeds.
- Symlinks inside `dirs` are visible. Relative links are preserved only when their resolved target stays inside the source. Absolute, broken, and escaping links are exposed as self-loops; this keeps them listable, copyable as links, and deletable without allowing path traversal outside the source tree.
- Virtual directories use mode `0755` so kernel permission checks allow `unlink` and `rmdir` callbacks to run. Creation and modification remain disabled at the FUSE operation layer.

## Testing strategy

- Unit tests exercise path mapping, classification, date boundaries, path validation, and inode behavior without requiring a mounted filesystem.
- A Linux integration script mounts a temporary source, verifies live views/read/delete/GNU `mv`, and unmounts cleanly. It skips with a clear message when `/dev/fuse` is unavailable.

# downloads-fuse

`downloads-fuse` is a Linux FUSE filesystem that presents files from `~/Downloads` through virtual category directories mounted at `~/dl`.

## Views

| Path                   | Contents                                                          |
| ---------------------- | ----------------------------------------------------------------- |
| `today/`, `yesterday/` | Top-level regular files from the corresponding local calendar day |
| `week/`, `month/`      | Top-level regular files modified in the last 7 or 30 rolling days |
| `pdf/`                 | Top-level `.pdf` files                                            |
| `images/`              | Top-level images and videos                                       |
| `zip/`                 | Top-level archives listed                                         |
| `dirs/`                | Top-level directories and their full recursive contents           |

Extension matching is case-insensitive. Views update on every lookup and listing, so changes made directly in the source appear without remounting.

## Requirements

- Linux with FUSE 3 and an accessible `/dev/fuse` device

## Build

With Nix:

```bash
nix build
```

The binary is available as `result/bin/downloads-fuse`. For development:

```bash
nix develop
cargo test
cargo clippy --all-targets -- -D warnings
```

Without Nix, install a stable Rust toolchain and FUSE 3 (`fusermount3`), then run:

```bash
cargo build --release
```

## Mount and unmount

Create the default mount point once, then start the filesystem:

```bash
mkdir -p ~/dl
nix run
```

`nix run` stays in the foreground and must remain running. In another terminal, verify the mount:

```bash
ls ~/dl
ls ~/dl/zip
```

The defaults are `~/Downloads` and `~/dl`. Override either path when testing:

```bash
nix run . -- --source /absolute/source --mount /absolute/mount-point
```

To stop it, press <kbd>Ctrl</kbd>+<kbd>C</kbd> in the terminal running `nix run`. Alternatively, unmount it from another terminal:

```bash
fusermount3 -u ~/dl
```

The source and mount point must already exist. Both paths are resolved at startup, and the mount point is rejected if it is inside the source directory.

## Supported operations and limits

- `ls`, `find`, `cat`, `cp`, and `cd` work through the projected views.
- `rm` deletes the original. `rm -r` works below `dirs/` and cannot delete the source root.
- GNU `mv` out of the mount works through its cross-filesystem copy-then-delete fallback for files and directories.
- Creating, editing, or moving entries into the mount is intentionally unsupported.
- Symlinks below `dirs/` are listed. Relative links resolving inside the source work normally; absolute, broken, or escaping links are returned as safe self-loops so traversal cannot leave the source tree. They can still be copied as links or deleted.
- No index is kept. Entries that disappear during an operation return the underlying Linux error.

## Integration test

The test creates temporary source and mount directories and performs real FUSE operations:

```bash
nix develop --command cargo build
nix develop --command tests/integration.sh target/debug/downloads-fuse
```

It prints `SKIP` and exits successfully when `/dev/fuse`, `fusermount3`, or `mountpoint` is unavailable.

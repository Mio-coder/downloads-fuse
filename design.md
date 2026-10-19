# Specification: `dl` — FUSE Filesystem for Downloads

## 1. Goal

Implement a Linux FUSE filesystem mounted at `~/dl` that provides dynamic, categorized views of files and directories in `~/Downloads`.

Files must not be duplicated. All views reference the original files. File operations must affect the originals.

## 2. Directory structure

```text
~/dl/
├── today/       # Files modified today
├── yesterday/   # Files modified yesterday
├── week/        # Files modified within the last 7 days
├── month/       # Files modified within the last 30 days
├── pdf/         # .pdf files
├── images/      # Images and videos
├── zip/         # Archives
└── dirs/        # Directories in ~/Downloads
```

Filters are not mutually exclusive. One file can appear in multiple views.

## 3. Example

Source:

```text
~/Downloads/
├── a.zip            # Modified today
└── b/
    └── c.png        # Modified 2 months ago
```

Virtual filesystem:

```text
~/dl/
├── today/
│   └── a.zip
├── yesterday/
├── week/
│   └── a.zip
├── month/
│   └── a.zip
├── pdf/
├── images/
├── zip/
│   └── a.zip
└── dirs/
    └── b/
        └── c.png
```

**Important rules:**

- Only top-level regular files in `~/Downloads` are classified into date/type views.
- Subdirectories appear under `dirs/`.
- Directories under `dirs/` expose their complete original contents, recursively, without additional filtering.
- Directory listing order is unspecified.

## 4. File operations

### Reading

Standard read operations must work:

```bash
cat ~/dl/today/file.txt
cp ~/dl/pdf/document.pdf .
cd ~/dl/dirs/project
```

### Deleting

Deleting a virtual file deletes the original file:

```bash
rm ~/dl/today/a.zip
```

Equivalent effect:

```bash
rm ~/Downloads/a.zip
```

Deleting a file through any view removes it from all views.

Directories support recursive deletion:

```bash
rm -r ~/dl/dirs/project
```

This deletes `~/Downloads/project` and its contents.

### Moving files out

Moving a file out of `~/dl` must remove it from Downloads and place it at the destination:

```bash
mv ~/dl/today/a.zip .
```

Expected result:

```text
~/Downloads/a.zip   # No longer exists
./a.zip              # Exists
```

Moving directories must also work:

```bash
mv ~/dl/dirs/project .
```

Cross-filesystem moves may use copy-then-delete semantics. The original must not be deleted if copying fails.

### Writing

Category views (`today`, `pdf`, `zip`, etc.) are read-only regarding file creation and modification.

## 5. File types

File classification uses case-insensitive extensions.

| Category | Extensions                                                                                                 |
| -------- | ---------------------------------------------------------------------------------------------------------- |
| `pdf`    | `.pdf`                                                                                                     |
| `images` | `.jpg`, `.jpeg`, `.png`, `.gif`, `.webp`, `.svg`, `.bmp`, `.heic`, `.mp4`, `.mkv`, `.mov`, `.avi`, `.webm` |
| `zip`    | `.zip`, `.tar`, `.gz`, `.tgz`, `.bz2`, `.xz`, `.7z`, `.rar`, `.zst`                                        |

The `zip` category includes archives other than ZIP files.

Files with unrecognized extensions remain accessible in date-based views.

## 7. Date filtering

Use filesystem modification timestamps (`mtime`) and local time.

- `today`: from local midnight today
- `yesterday`: previous calendar day
- `week`: rolling last 7 × 24 hours
- `month`: rolling last 30 × 24 hours

Date filters must update as time passes, without remounting.

## 7. FUSE implementation requirements

- Target Linux.
- Use FUSE 3.
- Default mount point: `~/dl`.
- Default source directory: `~/Downloads`.
- Resolve both paths to absolute paths at startup.
- Do not copy files to implement virtual views.
- Implement directory listing, lookup, attributes, reading, and deletion.
- Preserve original file metadata where practical.
- Support standard tools (`ls`, `find`, `cat`, `cp`, `mv`, `rm`, `cd`).
- Changes made directly to `~/Downloads` must become visible without remounting.
- Handle files disappearing or changing during operations gracefully.
- Do not follow symlinks outside the source tree for mutating operations.
- Never allow virtual paths to escape the configured source directory.
- Never let FUSE delete or modify the source root directory itself.
- Prevent mounting `~/dl` inside the source directory.

### Rename and move behavior

FUSE is a separate mount, even when backed by files on the same underlying disk.

Consequently, moving files between `~/dl` and a normal filesystem path typically produces `EXDEV` and userspace `mv` performs copy-then-delete.

The implementation must support the filesystem operations needed for this fallback, including opening, reading, `unlink`, and `rmdir`.

**Note:** A FUSE filesystem cannot guarantee the behavior of every `mv` implementation. GNU coreutils `mv` should be included in integration tests.

## 8. Non-goals (initial version)

- No recursive classification of files inside Downloads subdirectories.
- No background indexing or persistent database.
- No copying or moving files into category views.
- No writing or editing files through category views.
- No automatic file organization or moving files between source directories.
- No graphical UI.

## 9. Deliverables

The coding agent should produce:

- Working FUSE 3 implementation.
- Build and installation instructions.
- Nix flake.
- Commands to mount and unmount.
- Automated integration tests using a temporary Downloads directory.
- README describing the categories, limits, and supported operations.

Prefer a minimal implementation without unnecessary dependencies or infrastructure.

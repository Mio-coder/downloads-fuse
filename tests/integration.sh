#!/usr/bin/env bash
set -euo pipefail

binary=${1:-target/debug/downloads-fuse}

if [[ ! -c /dev/fuse ]] || [[ ! -r /dev/fuse ]] || [[ ! -w /dev/fuse ]]; then
  echo "SKIP: /dev/fuse is not available to this user"
  exit 0
fi
if ! command -v fusermount3 >/dev/null || ! command -v mountpoint >/dev/null; then
  echo "SKIP: fusermount3 and mountpoint are required"
  exit 0
fi

work=$(mktemp -d)
source_dir="$work/Downloads"
mount_dir="$work/dl"
destination="$work/destination"
mkdir -p "$source_dir/project" "$source_dir/doomed" "$mount_dir" "$destination"

cleanup() {
  if mountpoint -q "$mount_dir"; then
    fusermount3 -u "$mount_dir" || true
  fi
  if [[ -n ${mount_pid:-} ]]; then
    kill "$mount_pid" 2>/dev/null || true
    wait "$mount_pid" 2>/dev/null || true
  fi
  rm -rf "$work"
}
trap cleanup EXIT

printf 'today\n' >"$source_dir/today.txt"
printf 'pdf\n' >"$source_dir/report.PDF"
printf 'image\n' >"$source_dir/photo.JPG"
printf 'archive\n' >"$source_dir/bundle.zst"
printf 'yesterday\n' >"$source_dir/yesterday.txt"
printf 'nested\n' >"$source_dir/project/nested.txt"
printf 'delete recursively\n' >"$source_dir/doomed/file.txt"
printf 'old\n' >"$source_dir/old.txt"
touch -d '40 days ago' "$source_dir/old.txt"
touch -d 'yesterday 12:00' "$source_dir/yesterday.txt"
ln -s /etc/passwd "$source_dir/project/outside-link"

"$binary" --source "$source_dir" --mount "$mount_dir" >"$work/mount.log" 2>&1 &
mount_pid=$!
for _ in {1..50}; do
  mountpoint -q "$mount_dir" && break
  kill -0 "$mount_pid" 2>/dev/null || {
    cat "$work/mount.log"
    exit 1
  }
  sleep 0.1
done
mountpoint -q "$mount_dir"

test -f "$mount_dir/today/today.txt"
test -f "$mount_dir/yesterday/yesterday.txt"
test -f "$mount_dir/week/yesterday.txt"
test -f "$mount_dir/month/yesterday.txt"
test -f "$mount_dir/pdf/report.PDF"
test -f "$mount_dir/images/photo.JPG"
test -f "$mount_dir/zip/bundle.zst"
test ! -e "$mount_dir/month/old.txt"
cmp "$source_dir/project/nested.txt" "$mount_dir/dirs/project/nested.txt"

printf 'live\n' >"$source_dir/live.pdf"
test -f "$mount_dir/pdf/live.pdf"
if touch "$mount_dir/pdf/blocked.pdf" 2>/dev/null; then
  echo "category view unexpectedly allowed file creation"
  exit 1
fi
if printf 'changed\n' 2>/dev/null >"$mount_dir/pdf/live.pdf"; then
  echo "category view unexpectedly allowed file modification"
  exit 1
fi
grep -qx live "$source_dir/live.pdf"

rm "$mount_dir/today/today.txt"
test ! -e "$source_dir/today.txt"

mv "$mount_dir/pdf/report.PDF" "$destination/report.PDF"
test ! -e "$source_dir/report.PDF"
test -f "$destination/report.PDF"

rm -r "$mount_dir/dirs/doomed"
test ! -e "$source_dir/doomed"

mv "$mount_dir/dirs/project" "$destination/project"
test ! -e "$source_dir/project"
test -f "$destination/project/nested.txt"
test -L "$destination/project/outside-link"

echo "PASS: live views, reads, deletes, recursive deletes, and GNU mv"

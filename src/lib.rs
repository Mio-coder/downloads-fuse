use chrono::{DateTime, Days, Local};
use fuser::{
    FileAttr, FileType, Filesystem, KernelConfig, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, Request,
};
use libc::{EACCES, EINVAL, EIO, EISDIR, ENOENT, ENOTDIR, EPERM};
use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, Metadata};
use std::hash::{Hash, Hasher};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ROOT_INO: u64 = 1;
const TTL: Duration = Duration::from_secs(0);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum View {
    Today,
    Yesterday,
    Week,
    Month,
    Pdf,
    Images,
    Zip,
    Dirs,
}

impl View {
    pub const ALL: [Self; 8] = [
        Self::Today,
        Self::Yesterday,
        Self::Week,
        Self::Month,
        Self::Pdf,
        Self::Images,
        Self::Zip,
        Self::Dirs,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Yesterday => "yesterday",
            Self::Week => "week",
            Self::Month => "month",
            Self::Pdf => "pdf",
            Self::Images => "images",
            Self::Zip => "zip",
            Self::Dirs => "dirs",
        }
    }

    fn inode(self) -> u64 {
        2 + Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap() as u64
    }

    fn from_inode(ino: u64) -> Option<Self> {
        Self::ALL.get(ino.checked_sub(2)? as usize).copied()
    }

    fn from_name(name: &OsStr) -> Option<Self> {
        Self::ALL.into_iter().find(|view| name == view.name())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Node {
    Entry { view: View, relative: PathBuf },
}

#[derive(Default)]
struct Inodes {
    by_inode: HashMap<u64, Node>,
    by_node: HashMap<Node, u64>,
}

impl Inodes {
    fn inode_for(&mut self, node: Node) -> u64 {
        if let Some(ino) = self.by_node.get(&node) {
            return *ino;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        node.hash(&mut hasher);
        let mut ino = hasher.finish().max(10);
        while matches!(self.by_inode.get(&ino), Some(existing) if existing != &node) {
            ino = ino.wrapping_add(1).max(10);
        }
        self.by_inode.insert(ino, node.clone());
        self.by_node.insert(node, ino);
        ino
    }
}

pub struct DownloadsFs {
    source: PathBuf,
    inodes: Mutex<Inodes>,
    handles: Mutex<HashMap<u64, File>>,
    next_handle: AtomicU64,
}

impl DownloadsFs {
    pub fn new(source: PathBuf) -> Self {
        Self {
            source,
            inodes: Mutex::new(Inodes::default()),
            handles: Mutex::new(HashMap::new()),
            next_handle: AtomicU64::new(1),
        }
    }

    fn node(&self, ino: u64) -> Option<Node> {
        self.inodes.lock().ok()?.by_inode.get(&ino).cloned()
    }

    fn inode_for(&self, view: View, relative: PathBuf) -> Result<u64, i32> {
        self.inodes
            .lock()
            .map_err(|_| EIO)
            .map(|mut inodes| inodes.inode_for(Node::Entry { view, relative }))
    }

    fn path_for(&self, relative: &Path) -> Result<PathBuf, i32> {
        if !valid_relative(relative) {
            return Err(EINVAL);
        }
        Ok(self.source.join(relative))
    }

    fn entry(&self, ino: u64) -> Result<(View, PathBuf, PathBuf, Metadata), i32> {
        let Node::Entry { view, relative } = self.node(ino).ok_or(ENOENT)?;
        let path = self.path_for(&relative)?;
        let metadata = fs::symlink_metadata(&path).map_err(errno)?;
        if !self.visible(view, &relative, &metadata, SystemTime::now()) {
            return Err(ENOENT);
        }
        Ok((view, relative, path, metadata))
    }

    fn visible(&self, view: View, relative: &Path, metadata: &Metadata, now: SystemTime) -> bool {
        if !entry_visible(view, relative, metadata, now) {
            return false;
        }
        if metadata.file_type().is_symlink() {
            return fs::canonicalize(self.source.join(relative))
                .is_ok_and(|target| target.starts_with(&self.source));
        }
        true
    }

    fn child(&self, parent: u64, name: &OsStr) -> Result<(View, PathBuf), i32> {
        if !valid_name(name) {
            return Err(EINVAL);
        }
        if let Some(view) = View::from_inode(parent) {
            return Ok((view, PathBuf::from(name)));
        }
        let Node::Entry { view, relative } = self.node(parent).ok_or(ENOENT)?;
        if view != View::Dirs {
            return Err(ENOTDIR);
        }
        let parent_path = self.path_for(&relative)?;
        if !fs::symlink_metadata(parent_path).map_err(errno)?.is_dir() {
            return Err(ENOTDIR);
        }
        Ok((view, relative.join(name)))
    }

    fn list(&self, ino: u64) -> Result<Vec<(u64, FileType, OsString)>, i32> {
        if ino == ROOT_INO {
            return Ok(View::ALL
                .into_iter()
                .map(|view| {
                    (
                        view.inode(),
                        FileType::Directory,
                        OsString::from(view.name()),
                    )
                })
                .collect());
        }

        let (view, base) = if let Some(view) = View::from_inode(ino) {
            (view, PathBuf::new())
        } else {
            let (view, relative, path, metadata) = self.entry(ino)?;
            if view != View::Dirs || !metadata.is_dir() {
                return Err(ENOTDIR);
            }
            // Refuse to traverse a directory that was swapped for a symlink.
            if metadata.file_type().is_symlink() {
                return Err(ENOTDIR);
            }
            debug_assert_eq!(path, self.source.join(&relative));
            (view, relative)
        };

        let directory = self.path_for(&base)?;
        let now = SystemTime::now();
        let mut entries = Vec::new();
        for item in fs::read_dir(directory).map_err(errno)? {
            let item = match item {
                Ok(item) => item,
                Err(_) => continue,
            };
            let name = item.file_name();
            if !valid_name(&name) {
                continue;
            }
            let relative = base.join(&name);
            let metadata = match fs::symlink_metadata(item.path()) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if !self.visible(view, &relative, &metadata, now) {
                continue;
            }
            let ino = self.inode_for(view, relative)?;
            entries.push((ino, file_type(&metadata), name));
        }
        Ok(entries)
    }
}

impl Filesystem for DownloadsFs {
    fn init(&mut self, _req: &Request<'_>, config: &mut KernelConfig) -> Result<(), libc::c_int> {
        config
            .add_capabilities(fuser::consts::FUSE_READDIRPLUS_AUTO)
            .map_err(|_| EIO)?;
        Ok(())
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == ROOT_INO {
            match View::from_name(name) {
                Some(view) => reply.entry(&TTL, &virtual_dir_attr(view.inode()), 0),
                None => reply.error(ENOENT),
            }
            return;
        }

        let result = (|| {
            let (view, relative) = self.child(parent, name)?;
            let path = self.path_for(&relative)?;
            let metadata = fs::symlink_metadata(path).map_err(errno)?;
            if !self.visible(view, &relative, &metadata, SystemTime::now()) {
                return Err(ENOENT);
            }
            let ino = self.inode_for(view, relative)?;
            Ok(attr(ino, &metadata))
        })();
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, 0),
            Err(error) => reply.error(error),
        }
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        if ino == ROOT_INO || View::from_inode(ino).is_some() {
            reply.attr(&TTL, &virtual_dir_attr(ino));
            return;
        }
        match self.entry(ino) {
            Ok((_, _, _, metadata)) => reply.attr(&TTL, &attr(ino, &metadata)),
            Err(error) => reply.error(error),
        }
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        match self.entry(ino).and_then(|(_, _, path, metadata)| {
            if !metadata.file_type().is_symlink() {
                return Err(EINVAL);
            }
            fs::read_link(path).map_err(errno)
        }) {
            Ok(target) => reply.data(target.as_os_str().as_bytes()),
            Err(error) => reply.error(error),
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        let access_mode = flags & libc::O_ACCMODE;
        if access_mode != libc::O_RDONLY {
            reply.error(EACCES);
            return;
        }
        let result = self.entry(ino).and_then(|(_, _, path, metadata)| {
            if metadata.is_dir() {
                return Err(EISDIR);
            }
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map_err(errno)?;
            let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
            self.handles.lock().map_err(|_| EIO)?.insert(handle, file);
            Ok(handle)
        });
        match result {
            Ok(handle) => reply.opened(handle, 0),
            Err(error) => reply.error(error),
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        if offset < 0 {
            reply.error(EINVAL);
            return;
        }
        let mut buffer = vec![0; size as usize];
        let result = self.handles.lock().map_err(|_| EIO).and_then(|handles| {
            handles
                .get(&fh)
                .ok_or(ENOENT)?
                .read_at(&mut buffer, offset as u64)
                .map_err(errno)
        });
        match result {
            Ok(read) => reply.data(&buffer[..read]),
            Err(error) => reply.error(error),
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        _ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        match self.handles.lock() {
            Ok(mut handles) => {
                handles.remove(&fh);
                reply.ok();
            }
            Err(_) => reply.error(EIO),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let mut entries = vec![
            (ino, FileType::Directory, OsString::from(".")),
            (
                parent_inode(self, ino),
                FileType::Directory,
                OsString::from(".."),
            ),
        ];
        match self.list(ino) {
            Ok(children) => entries.extend(children),
            Err(error) => {
                reply.error(error);
                return;
            }
        }
        for (index, (entry_ino, kind, name)) in
            entries.into_iter().enumerate().skip(offset.max(0) as usize)
        {
            if reply.add(entry_ino, (index + 1) as i64, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = self.child(parent, name).and_then(|(view, relative)| {
            let path = self.path_for(&relative)?;
            let metadata = fs::symlink_metadata(&path).map_err(errno)?;
            if !self.visible(view, &relative, &metadata, SystemTime::now()) {
                return Err(ENOENT);
            }
            if metadata.is_dir() {
                return Err(EISDIR);
            }
            fs::remove_file(path).map_err(errno)
        });
        respond_empty(reply, result);
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        let result = self.child(parent, name).and_then(|(view, relative)| {
            if view != View::Dirs {
                return Err(EPERM);
            }
            let path = self.path_for(&relative)?;
            let metadata = fs::symlink_metadata(&path).map_err(errno)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(ENOTDIR);
            }
            fs::remove_dir(path).map_err(errno)
        });
        respond_empty(reply, result);
    }
}

fn parent_inode(fs: &DownloadsFs, ino: u64) -> u64 {
    if ino == ROOT_INO {
        return ROOT_INO;
    }
    if View::from_inode(ino).is_some() {
        return ROOT_INO;
    }
    let Some(Node::Entry { view, relative }) = fs.node(ino) else {
        return ROOT_INO;
    };
    match relative.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => fs
            .inode_for(view, parent.to_path_buf())
            .unwrap_or(view.inode()),
        _ => view.inode(),
    }
}

fn respond_empty(reply: ReplyEmpty, result: Result<(), i32>) {
    match result {
        Ok(()) => reply.ok(),
        Err(error) => reply.error(error),
    }
}

fn valid_name(name: &OsStr) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.as_bytes().contains(&b'/')
}

fn valid_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

pub fn entry_visible(view: View, relative: &Path, metadata: &Metadata, now: SystemTime) -> bool {
    let top_level = relative.components().count() == 1;
    if view == View::Dirs {
        return !top_level || metadata.is_dir();
    }
    if !top_level || !metadata.is_file() {
        return false;
    }

    let extension = relative
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase);
    match view {
        View::Pdf => extension.as_deref() == Some("pdf"),
        View::Images => matches!(
            extension.as_deref(),
            Some(
                "jpg"
                    | "jpeg"
                    | "png"
                    | "gif"
                    | "webp"
                    | "svg"
                    | "bmp"
                    | "heic"
                    | "mp4"
                    | "mkv"
                    | "mov"
                    | "avi"
                    | "webm"
            )
        ),
        View::Zip => matches!(
            extension.as_deref(),
            Some("zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "zst")
        ),
        View::Today | View::Yesterday | View::Week | View::Month => {
            let Ok(modified) = metadata.modified() else {
                return false;
            };
            match view {
                View::Today => local_date(modified) == Some(Local::now().date_naive()),
                View::Yesterday => Local::now()
                    .date_naive()
                    .checked_sub_days(Days::new(1))
                    .is_some_and(|date| local_date(modified) == Some(date)),
                View::Week => now
                    .duration_since(modified)
                    .is_ok_and(|age| age <= Duration::from_secs(7 * 24 * 60 * 60)),
                View::Month => now
                    .duration_since(modified)
                    .is_ok_and(|age| age <= Duration::from_secs(30 * 24 * 60 * 60)),
                _ => unreachable!(),
            }
        }
        View::Dirs => unreachable!(),
    }
}

fn local_date(time: SystemTime) -> Option<chrono::NaiveDate> {
    Some(DateTime::<Local>::from(time).date_naive())
}

fn errno(error: io::Error) -> i32 {
    error.raw_os_error().unwrap_or(EIO)
}

fn file_type(metadata: &Metadata) -> FileType {
    let kind = metadata.file_type();
    if kind.is_dir() {
        FileType::Directory
    } else if kind.is_symlink() {
        FileType::Symlink
    } else if kind.is_file() {
        FileType::RegularFile
    } else {
        match metadata.mode() & libc::S_IFMT {
            libc::S_IFBLK => FileType::BlockDevice,
            libc::S_IFCHR => FileType::CharDevice,
            libc::S_IFIFO => FileType::NamedPipe,
            libc::S_IFSOCK => FileType::Socket,
            _ => FileType::RegularFile,
        }
    }
}

fn system_time(seconds: i64, nanoseconds: i64) -> SystemTime {
    let total_nanos = i128::from(seconds) * 1_000_000_000 + i128::from(nanoseconds);
    if total_nanos >= 0 {
        UNIX_EPOCH
            + Duration::new(
                (total_nanos / 1_000_000_000) as u64,
                (total_nanos % 1_000_000_000) as u32,
            )
    } else {
        let magnitude = -total_nanos;
        UNIX_EPOCH
            - Duration::new(
                (magnitude / 1_000_000_000) as u64,
                (magnitude % 1_000_000_000) as u32,
            )
    }
}

fn attr(ino: u64, metadata: &Metadata) -> FileAttr {
    FileAttr {
        ino,
        size: metadata.size(),
        blocks: metadata.blocks(),
        atime: system_time(metadata.atime(), metadata.atime_nsec()),
        mtime: system_time(metadata.mtime(), metadata.mtime_nsec()),
        ctime: system_time(metadata.ctime(), metadata.ctime_nsec()),
        crtime: UNIX_EPOCH,
        kind: file_type(metadata),
        perm: (metadata.mode() & 0o7777) as u16,
        nlink: metadata.nlink() as u32,
        uid: metadata.uid(),
        gid: metadata.gid(),
        rdev: metadata.rdev() as u32,
        flags: 0,
        blksize: metadata.blksize() as u32,
    }
}

fn virtual_dir_attr(ino: u64) -> FileAttr {
    FileAttr {
        ino,
        size: 0,
        blocks: 0,
        atime: UNIX_EPOCH,
        mtime: UNIX_EPOCH,
        ctime: UNIX_EPOCH,
        crtime: UNIX_EPOCH,
        kind: FileType::Directory,
        perm: 0o555,
        nlink: 2,
        uid: unsafe { libc::geteuid() },
        gid: unsafe { libc::getegid() },
        rdev: 0,
        flags: 0,
        blksize: 4096,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::OpenOptions;
    use std::time::Duration;

    fn temp_file(name: &str) -> (PathBuf, Metadata) {
        let dir = std::env::temp_dir().join(format!(
            "downloads-fuse-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        let metadata = fs::metadata(&path).unwrap();
        (dir, metadata)
    }

    #[test]
    fn classifies_extensions_case_insensitively() {
        let (dir, metadata) = temp_file("PHOTO.JPEG");
        assert!(entry_visible(
            View::Images,
            Path::new("PHOTO.JPEG"),
            &metadata,
            SystemTime::now()
        ));
        assert!(!entry_visible(
            View::Pdf,
            Path::new("PHOTO.JPEG"),
            &metadata,
            SystemTime::now()
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rolling_windows_reject_future_files() {
        let (dir, metadata) = temp_file("sample.txt");
        let before_mtime = metadata.modified().unwrap() - Duration::from_secs(1);
        assert!(!entry_visible(
            View::Week,
            Path::new("sample.txt"),
            &metadata,
            before_mtime
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dirs_only_accepts_directories_at_top_level() {
        let (dir, metadata) = temp_file("sample.txt");
        assert!(!entry_visible(
            View::Dirs,
            Path::new("sample.txt"),
            &metadata,
            SystemTime::now()
        ));
        assert!(entry_visible(
            View::Dirs,
            Path::new("folder/sample.txt"),
            &metadata,
            SystemTime::now()
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn relative_paths_cannot_escape() {
        assert!(valid_relative(Path::new("folder/file")));
        assert!(!valid_relative(Path::new("../file")));
        assert!(!valid_relative(Path::new("/tmp/file")));
        assert!(!valid_name(OsStr::new("..")));
    }

    #[test]
    fn inode_mapping_is_stable_and_view_specific() {
        let mut inodes = Inodes::default();
        let today = Node::Entry {
            view: View::Today,
            relative: PathBuf::from("x"),
        };
        let pdf = Node::Entry {
            view: View::Pdf,
            relative: PathBuf::from("x"),
        };
        assert_eq!(inodes.inode_for(today.clone()), inodes.inode_for(today));
        assert_ne!(
            inodes.inode_for(pdf.clone()),
            inodes.inode_for(Node::Entry {
                view: View::Today,
                relative: PathBuf::from("x")
            })
        );
    }
}

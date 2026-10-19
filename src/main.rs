use clap::Parser;
use downloads_fuse::DownloadsFs;
use fuser::MountOption;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// Directory whose contents are projected.
    #[arg(long, value_name = "PATH")]
    source: Option<PathBuf>,

    /// Directory on which to mount the projection.
    #[arg(long, value_name = "PATH")]
    mount: Option<PathBuf>,
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("downloads-fuse: {error}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set; pass --source and --mount".to_owned())?;
    let source = absolute_existing(
        cli.source.unwrap_or_else(|| home.join("Downloads")),
        "source",
    )?;
    let mount = absolute_existing(cli.mount.unwrap_or_else(|| home.join("dl")), "mount point")?;

    if !source.is_dir() {
        return Err(format!("source is not a directory: {}", source.display()));
    }
    if !mount.is_dir() {
        return Err(format!(
            "mount point is not a directory: {}",
            mount.display()
        ));
    }
    if mount.starts_with(&source) {
        return Err(format!(
            "mount point must not be inside source directory: {}",
            source.display()
        ));
    }

    let options = vec![
        MountOption::FSName("downloads-fuse".to_owned()),
        MountOption::Subtype("downloads".to_owned()),
        MountOption::DefaultPermissions,
        MountOption::NoAtime,
    ];
    let filesystem = DownloadsFs::new(source);
    fuser::mount2(filesystem, mount, &options).map_err(display_io)
}

fn absolute_existing(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    fs::canonicalize(&path)
        .map_err(|error| format!("cannot resolve {label} {}: {error}", path.display()))
}

fn display_io(error: std::io::Error) -> String {
    error.to_string()
}

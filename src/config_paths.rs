//! Preferences, durable data, and disposable media use separate XDG roots.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub fn paths(filename: &str) -> Option<(PathBuf, PathBuf)> {
    #[cfg(target_os = "windows")]
    let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let appdata = None;
    let base = config_base(
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        appdata,
        std::env::var_os("HOME").map(PathBuf::from),
    )?;
    Some((
        base.join("rdnmb").join(filename),
        base.join("nmbxd").join(filename),
    ))
}

pub(crate) fn data_path(filename: &str) -> Option<PathBuf> {
    storage_path("XDG_DATA_HOME", ".local/share", filename)
}

pub fn cache_path(filename: &str) -> Option<PathBuf> {
    storage_path("XDG_CACHE_HOME", ".cache", filename)
}

fn storage_path(variable: &str, home_suffix: &str, filename: &str) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let home_suffix = {
        let _ = home_suffix;
        "AppData/Local"
    };
    #[cfg(target_os = "windows")]
    let local_appdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(target_os = "windows"))]
    let local_appdata = None;
    let home = std::env::var_os("HOME");
    #[cfg(target_os = "windows")]
    let home = home.or_else(|| std::env::var_os("USERPROFILE"));
    let home = home.map(PathBuf::from);
    storage_base(
        std::env::var_os(variable).map(PathBuf::from),
        local_appdata,
        home,
        home_suffix,
    )
    .map(|base| base.join("rdnmb").join(filename))
}

fn storage_base(
    xdg: Option<PathBuf>,
    local_appdata: Option<PathBuf>,
    home: Option<PathBuf>,
    home_suffix: &str,
) -> Option<PathBuf> {
    xdg.filter(|path| path.is_absolute())
        .or_else(|| local_appdata.filter(|path| path.is_absolute()))
        .or_else(|| {
            home.filter(|path| path.is_absolute())
                .map(|path| path.join(home_suffix))
        })
}

pub(crate) fn create_private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// An unpublished file in the destination directory. Hard-link publication is
/// atomic and refuses to replace even a destination created by another process.
pub(crate) struct MigrationFile {
    path: PathBuf,
    pub(crate) file: fs::File,
}

impl MigrationFile {
    pub(crate) fn new(directory: &Path) -> io::Result<Self> {
        create_private_directory(directory)?;
        let path = directory.join(format!(".rdnmb-migrate-{}", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(Self {
            file: options.open(&path)?,
            path,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn publish(&self, destination: &Path) -> io::Result<()> {
        self.file.sync_all()?;
        match fs::hard_link(&self.path, destination) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for MigrationFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Merge the two former flat caches in preference order, retaining originals.
/// Files are fully copied before publication and existing destinations win.
pub fn migrate_media_cache(destination: &Path, sources: &[PathBuf]) -> io::Result<()> {
    let marker = destination.join(".legacy-migration-complete");
    match fs::symlink_metadata(&marker) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    for source in sources {
        if source == destination {
            continue;
        }
        let entries = match fs::read_dir(source) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            // Atomic download temporaries and other stray files are not cache entries.
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() != 64 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let target = destination.join(entry.file_name());
            match fs::symlink_metadata(&target) {
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let mut temporary = MigrationFile::new(destination)?;
            io::copy(&mut fs::File::open(entry.path())?, &mut temporary.file)?;
            temporary.publish(&target)?;
        }
    }
    MigrationFile::new(destination)?.publish(&marker)
}

fn config_base(
    xdg: Option<PathBuf>,
    appdata: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    xdg.filter(|path| path.is_absolute())
        .or_else(|| appdata.filter(|path| path.is_absolute()))
        .or_else(|| {
            home.filter(|path| !path.as_os_str().is_empty())
                .map(|path| path.join(".config"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("rdnmb-paths-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn storage_roots_follow_absolute_xdg_then_windows_then_home() {
        let root = std::env::temp_dir();
        let xdg = root.join("xdg");
        let local = root.join("localappdata");
        let home = root.join("home");
        for suffix in [".local/share", ".cache", "AppData/Local"] {
            assert_eq!(
                storage_base(
                    Some(xdg.clone()),
                    Some(local.clone()),
                    Some(home.clone()),
                    suffix
                ),
                Some(xdg.clone())
            );
            assert_eq!(
                storage_base(
                    Some("relative".into()),
                    Some(local.clone()),
                    Some(home.clone()),
                    suffix
                ),
                Some(local.clone())
            );
            assert_eq!(
                storage_base(Some(PathBuf::new()), None, Some(home.clone()), suffix),
                Some(home.join(suffix))
            );
        }
        assert_eq!(
            storage_base(None, None, Some("relative".into()), ".cache"),
            None
        );
    }

    #[test]
    fn cache_migration_preserves_destinations_uses_source_order_and_runs_once() {
        let root = TestDirectory::new();
        let destination = root.0.join("cache");
        let current = root.0.join("rdnmb");
        let legacy = root.0.join("nmbxd");
        for directory in [&destination, &current, &legacy] {
            fs::create_dir_all(directory).unwrap();
        }
        let occupied = "a".repeat(64);
        let preferred = "b".repeat(64);
        let fallback = "c".repeat(64);
        let directory = "d".repeat(64);
        fs::write(destination.join(&occupied), b"keep").unwrap();
        fs::create_dir(destination.join(&directory)).unwrap();
        for name in [&occupied, &preferred, &directory] {
            fs::write(current.join(name), b"current").unwrap();
        }
        for name in [&preferred, &fallback] {
            fs::write(legacy.join(name), b"legacy").unwrap();
        }
        fs::write(current.join(".unfinished.tmp"), b"incomplete").unwrap();
        let sources = [current.clone(), legacy.clone()];
        migrate_media_cache(&destination, &sources).unwrap();
        assert_eq!(fs::read(destination.join(&occupied)).unwrap(), b"keep");
        assert_eq!(fs::read(destination.join(&preferred)).unwrap(), b"current");
        assert_eq!(fs::read(destination.join(&fallback)).unwrap(), b"legacy");
        assert!(destination.join(&directory).is_dir());
        assert!(!destination.join(".unfinished.tmp").exists());
        assert!(current.join(&preferred).exists());
        assert!(legacy.join(&fallback).exists());
        fs::remove_file(destination.join(&preferred)).unwrap();
        migrate_media_cache(&destination, &sources).unwrap();
        assert!(
            !destination.join(&preferred).exists(),
            "cleared images must not return next launch"
        );
    }

    #[test]
    fn failed_cache_migration_remains_retryable() {
        let root = TestDirectory::new();
        let destination = root.0.join("cache");
        let source = root.0.join("source");
        fs::write(&source, b"not a directory").unwrap();
        assert!(migrate_media_cache(&destination, std::slice::from_ref(&source)).is_err());
        assert!(!destination.join(".legacy-migration-complete").exists());
        fs::remove_file(&source).unwrap();
        fs::create_dir(&source).unwrap();
        let name = "a".repeat(64);
        fs::write(source.join(&name), b"image").unwrap();
        migrate_media_cache(&destination, &[source]).unwrap();
        assert_eq!(fs::read(destination.join(&name)).unwrap(), b"image");
    }

    #[test]
    fn publishing_migration_never_overwrites_a_concurrent_destination() {
        use std::io::Write;
        let root = TestDirectory::new();
        let destination = root.0.join("destination");
        let mut temporary = MigrationFile::new(&root.0).unwrap();
        temporary.file.write_all(b"snapshot").unwrap();
        let temporary_path = temporary.path().to_owned();
        fs::write(&destination, b"winner").unwrap();
        temporary.publish(&destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"winner");
        drop(temporary);
        assert!(!temporary_path.exists());
    }

    #[cfg(unix)]
    #[test]
    fn cache_migration_does_not_replace_a_dangling_symlink() {
        use std::os::unix::fs::symlink;
        let root = TestDirectory::new();
        let source = root.0.join("source");
        let destination = root.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        let name = "a".repeat(64);
        fs::write(source.join(&name), b"image").unwrap();
        symlink(root.0.join("missing"), destination.join(&name)).unwrap();
        migrate_media_cache(&destination, &[source]).unwrap();
        assert!(
            fs::symlink_metadata(destination.join(name))
                .unwrap()
                .is_symlink()
        );
    }

    #[test]
    fn platform_base_precedence_ignores_empty_and_relative_overrides() {
        let base = std::env::temp_dir();
        let xdg = base.join("xdg");
        let appdata = base.join("appdata");
        let home = base.join("home");
        assert_eq!(
            config_base(Some(xdg.clone()), Some(appdata.clone()), Some(home.clone())),
            Some(xdg)
        );
        assert_eq!(
            config_base(
                Some("relative".into()),
                Some(appdata.clone()),
                Some(home.clone())
            ),
            Some(appdata)
        );
        assert_eq!(
            config_base(
                Some(PathBuf::new()),
                Some("relative".into()),
                Some(home.clone())
            ),
            Some(home.join(".config"))
        );
        assert_eq!(config_base(None, None, Some(PathBuf::new())), None);
    }
}

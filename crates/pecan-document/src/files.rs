//! Filesystem conventions shared by loading, saving and local resource resolution.
use std::path::{Path, PathBuf};

/// Resolve links and `..` into the identity used for tabs, history and saving.
///
/// On Windows this drops the `\\?\` verbatim prefix whenever an ordinary path names the
/// same file, so the identity can be shown to users and handed to the shell.
///
/// # Errors
/// Returns the I/O error from resolving a missing or inaccessible path.
pub fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Remove a Windows verbatim prefix when an ordinary path is equivalent.
#[must_use]
pub fn simplified(path: &Path) -> &Path {
    dunce::simplified(path)
}

/// Whether opening `path` would contact another machine or a device namespace.
///
/// Windows authenticates to SMB servers when a UNC path is opened, so a document that
/// names `\\host\share\image.png` could otherwise send the user's credentials to `host`.
/// Other platforms only reach network storage through local mount points.
#[must_use]
pub fn is_network_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        match path.components().next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(..) => true,
                // `\\?\Volume{GUID}\` names a local volume; other verbatim roots such as
                // `\\?\GLOBALROOT\Device\Mup\host` can reach the network redirector.
                Prefix::Verbatim(root) => !root
                    .to_string_lossy()
                    .get(..7)
                    .is_some_and(|start| start.eq_ignore_ascii_case("Volume{")),
                Prefix::Disk(_) | Prefix::VerbatimDisk(_) => false,
            },
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Whether a document whose relative resources resolve against `directory` may read `path`.
///
/// Local files are allowed. A network share is allowed only when it is the share that holds
/// the document itself, so documents opened from a share keep their relative images while a
/// document cannot make Windows connect to a different host.
#[must_use]
pub fn resource_allowed(directory: &Path, path: &Path) -> bool {
    if !is_network_path(path) {
        return true;
    }
    match (share(path), share(directory)) {
        (Some(resource), Some(document)) => {
            resource.0.eq_ignore_ascii_case(document.0)
                && resource.1.eq_ignore_ascii_case(document.1)
        }
        _ => false,
    }
}

#[cfg(windows)]
fn share(path: &Path) -> Option<(&std::ffi::OsStr, &std::ffi::OsStr)> {
    use std::path::{Component, Prefix};
    match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                Some((server, share))
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(not(windows))]
fn share(_: &Path) -> Option<(&std::ffi::OsStr, &std::ffi::OsStr)> {
    None
}

/// Atomically move a finished temporary file over `target`.
///
/// Windows refuses the replacement while a search indexer, antivirus scanner or sync
/// client briefly holds the destination open, so those transient failures are retried
/// for about two seconds before the error is returned.
///
/// # Errors
/// Returns the final I/O error; the temporary file is removed.
pub fn persist(mut temporary: tempfile::NamedTempFile, target: &Path) -> std::io::Result<()> {
    let mut delay = std::time::Duration::from_millis(25);
    for _ in 0..7 {
        match temporary.persist(target) {
            Ok(_) => return Ok(()),
            Err(error) if transient(&error.error) => {
                temporary = error.file;
                std::thread::sleep(delay);
                delay = (delay * 2).min(std::time::Duration::from_millis(500));
            }
            Err(error) => return Err(error.error),
        }
    }
    temporary
        .persist(target)
        .map(drop)
        .map_err(|error| error.error)
}

fn transient(error: &std::io::Error) -> bool {
    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION.
    cfg!(windows) && matches!(error.raw_os_error(), Some(5 | 32 | 33))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_replaces_the_target_and_removes_the_temporary_file() {
        use std::io::Write as _;
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("note.md");
        std::fs::write(&target, "before").unwrap();
        let mut temporary = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        temporary.write_all(b"after").unwrap();
        persist(temporary, &target).unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "after");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn local_resources_are_allowed() {
        let directory = tempfile::tempdir().unwrap();
        let canonical = canonicalize(directory.path()).unwrap();
        assert!(!is_network_path(&canonical));
        assert!(resource_allowed(&canonical, &canonical.join("image.png")));
        assert!(resource_allowed(
            &canonical,
            Path::new("/elsewhere/image.png")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_shares_are_limited_to_the_document_share() {
        let local = Path::new(r"C:\Docs");
        for path in [
            r"\\attacker\share\a.png",
            r"\\?\UNC\attacker\share\a.png",
            r"\\.\pipe\name",
            r"\\?\GLOBALROOT\Device\Mup\attacker\share\a.png",
        ] {
            assert!(is_network_path(Path::new(path)), "{path}");
            assert!(!resource_allowed(local, Path::new(path)), "{path}");
        }
        for path in [
            r"C:\Images\a.png",
            r"\\?\C:\Images\a.png",
            r"\\?\Volume{0}\a.png",
        ] {
            assert!(resource_allowed(local, Path::new(path)), "{path}");
        }
        let shared = Path::new(r"\\Files\Team\Docs");
        assert!(resource_allowed(
            shared,
            Path::new(r"\\files\team\Images\a.png")
        ));
        assert!(resource_allowed(
            shared,
            Path::new(r"\\?\UNC\FILES\TEAM\a.png")
        ));
        assert!(!resource_allowed(shared, Path::new(r"\\files\other\a.png")));
        assert!(!resource_allowed(
            shared,
            Path::new(r"\\attacker\team\a.png")
        ));
        assert_eq!(
            simplified(Path::new(r"\\?\C:\Users\me\a.md")),
            Path::new(r"C:\Users\me\a.md")
        );
    }
}

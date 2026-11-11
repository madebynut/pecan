//! A first-launch marker independent of resettable reading preferences.
use std::{fs, io, path::Path};

/// Whether this user has launched the application before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// Show the bundled welcome document automatically.
    First,
    /// Start with an empty reader unless a document was requested.
    Returning,
}

/// Record a launch without rewriting preferences or recent paths.
/// Existing installations count as returning users when upgrading to this feature.
/// Exclusive creation ensures simultaneous launches cannot both show the welcome page.
///
/// # Errors
/// Returns the filesystem error if the configuration directory or marker cannot be read
/// or created. Callers can still launch the reader without an automatic welcome page.
pub fn record_launch(directory: &Path) -> io::Result<Launch> {
    fs::create_dir_all(directory)?;
    let existing = directory.join("preferences.json").try_exists()?
        || directory.join("recent.json").try_exists()?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("launched"))
    {
        Ok(file) => {
            file.sync_all()?;
            Ok(if existing {
                Launch::Returning
            } else {
                Launch::First
            })
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(Launch::Returning),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welcome_is_first_launch_only_and_survives_reset_preferences() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("Pecan");
        assert_eq!(record_launch(&directory).unwrap(), Launch::First);
        assert_eq!(record_launch(&directory).unwrap(), Launch::Returning);
        crate::save_preferences(&directory.join("preferences.json"), &Default::default()).unwrap();
        assert_eq!(record_launch(&directory).unwrap(), Launch::Returning);
        fs::remove_file(directory.join("preferences.json")).unwrap();
        assert_eq!(record_launch(&directory).unwrap(), Launch::Returning);
    }

    #[test]
    fn upgrades_skip_welcome_and_preserve_existing_user_files() {
        for name in ["preferences.json", "recent.json"] {
            let directory = tempfile::tempdir().unwrap();
            fs::write(directory.path().join(name), "existing user data").unwrap();
            assert_eq!(record_launch(directory.path()).unwrap(), Launch::Returning);
            assert_eq!(
                fs::read_to_string(directory.path().join(name)).unwrap(),
                "existing user data"
            );
        }
    }

    #[test]
    fn concurrent_launches_show_welcome_once() {
        let directory = tempfile::tempdir().unwrap();
        let launches = std::thread::scope(|scope| {
            (0..4)
                .map(|_| scope.spawn(|| record_launch(directory.path()).unwrap()))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            launches
                .iter()
                .filter(|&&launch| launch == Launch::First)
                .count(),
            1
        );
    }

    #[test]
    fn inaccessible_directory_returns_error_without_a_marker() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("file");
        fs::write(&file, "file").unwrap();
        assert!(record_launch(&file).is_err());
    }
}

//! Which installed package owns an executable, read from the pacman local database.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use borbatop_core::{package_files, package_name_from_dir};

use crate::error::SourceError;

/// Pacman's local database: one directory per installed package.
pub const PACMAN_LOCAL_DB: &str = "/var/lib/pacman/local";
const FILES_LIST: &str = "files";
/// Only paths that can hold a running executable are indexed, to keep the index small.
const EXECUTABLE_PREFIXES: [&str; 6] = [
    "usr/bin/",
    "usr/sbin/",
    "usr/lib/",
    "usr/lib32/",
    "usr/libexec/",
    "opt/",
];
const INITIAL_INDEX_CAPACITY: usize = 65_536;

/// Maps executable paths to the package that installed them.
#[derive(Debug, Default)]
pub struct FileOwners {
    packages: Vec<Box<str>>,
    by_path: HashMap<Box<str>, usize>,
}

impl FileOwners {
    /// Builds the index from the pacman local database.
    ///
    /// A machine without pacman (no database directory) yields an empty index.
    ///
    /// # Arguments
    ///
    /// * `database` - directory holding one `name-version-release` folder per package
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::Io`] when the directory exists but cannot be read.
    pub fn load(database: &Path) -> Result<Self, SourceError> {
        let entries = match fs::read_dir(database) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(SourceError::io(database, error)),
        };

        let mut owners = Self {
            packages: Vec::new(),
            by_path: HashMap::with_capacity(INITIAL_INDEX_CAPACITY),
        };
        for entry in entries {
            let entry = entry.map_err(|error| SourceError::io(database, error))?;
            let directory = entry.file_name();
            let Some(package) = directory.to_str().and_then(package_name_from_dir) else {
                continue;
            };
            let list_path = entry.path().join(FILES_LIST);
            match fs::read_to_string(&list_path) {
                Ok(list) => owners.index_package(package, &list),
                // A package being upgraded right now may briefly lack its file list.
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(SourceError::io(list_path, error)),
            }
        }

        Ok(owners)
    }

    fn index_package(&mut self, package: &str, file_list: &str) {
        let slot = self.packages.len();
        let mut used = false;
        for path in package_files(file_list).filter(|path| {
            EXECUTABLE_PREFIXES
                .iter()
                .any(|prefix| path.starts_with(prefix))
        }) {
            self.by_path.insert(path.into(), slot);
            used = true;
        }
        if used {
            self.packages.push(package.into());
        }
    }

    /// Package that owns the file at `path`, if any.
    ///
    /// # Arguments
    ///
    /// * `path` - absolute path of an executable, as `/proc/[pid]/exe` reports it
    #[must_use]
    pub fn package_of(&self, path: &str) -> Option<&str> {
        let relative = path.strip_prefix('/')?;
        let slot = self.by_path.get(relative)?;
        self.packages.get(*slot).map(AsRef::as_ref)
    }

    /// Number of indexed paths.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    /// Whether nothing is indexed (not an Arch system, or an empty database).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn fixture_database(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("borbatop-owners-{}-{name}", std::process::id()));
        match fs::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => panic!("cannot clear fixture: {error}"),
        }
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn indexes_executables_and_ignores_docs() {
        let root = fixture_database("index");
        let bash = root.join("bash-5.3.20-1");
        fs::create_dir_all(&bash).unwrap();
        fs::write(
            bash.join(FILES_LIST),
            "%FILES%\nusr/\nusr/bin/\nusr/bin/bash\nusr/share/doc/bash/README\n",
        )
        .unwrap();
        let broken = root.join("half-installed-1-1");
        fs::create_dir_all(&broken).unwrap();
        fs::create_dir_all(root.join("ALPM_DB_VERSION")).unwrap();

        let owners = FileOwners::load(&root).unwrap();

        assert_eq!(owners.package_of("/usr/bin/bash"), Some("bash"));
        assert_eq!(owners.package_of("/usr/share/doc/bash/README"), None);
        assert_eq!(owners.package_of("usr/bin/bash"), None);
        assert_eq!(owners.len(), 1);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_database_is_an_empty_index() {
        let owners = FileOwners::load(Path::new("/nonexistent/borbatop/db")).unwrap();
        assert!(owners.is_empty());
    }

    #[test]
    fn real_database_resolves_the_shell() {
        let owners = FileOwners::load(Path::new(PACMAN_LOCAL_DB)).unwrap();
        if owners.is_empty() {
            return; // not an Arch machine
        }
        assert_eq!(owners.package_of("/usr/bin/bash"), Some("bash"));
    }
}

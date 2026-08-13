use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const MAX_CREDENTIAL_BYTES: u64 = 64 * 1024;

pub(crate) struct SecretCredential(String);

impl SecretCredential {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

pub(crate) trait CredentialStore: Send + Sync {
    fn read(&self) -> Result<Option<SecretCredential>, String>;
    fn write(&self, credential: &SecretCredential) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

pub(crate) struct FileCredentialStore {
    path: PathBuf,
}

impl FileCredentialStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn parent(&self) -> Result<&Path, String> {
        self.path
            .parent()
            .ok_or_else(|| "credential_store_write_failed".to_string())
    }

    #[cfg(unix)]
    fn ensure_private_directory(&self) -> Result<(), String> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let parent = self.parent()?;
        fs::create_dir_all(parent).map_err(|_| "credential_store_write_failed".to_string())?;
        let metadata = fs::symlink_metadata(parent)
            .map_err(|_| "credential_store_write_failed".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("credential_store_permissions_invalid".to_string());
        }
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .map_err(|_| "credential_store_write_failed".to_string())?;
        let metadata = fs::symlink_metadata(parent)
            .map_err(|_| "credential_store_write_failed".to_string())?;
        if metadata.mode() & 0o077 != 0 || metadata.uid() != unsafe { libc::geteuid() } {
            return Err("credential_store_permissions_invalid".to_string());
        }
        Ok(())
    }

    #[cfg(unix)]
    fn validate_private_file(&self) -> Result<Option<fs::Metadata>, String> {
        use std::os::unix::fs::MetadataExt;

        let parent = self
            .path
            .parent()
            .ok_or_else(|| "credential_store_read_failed".to_string())?;
        let parent_metadata = match fs::symlink_metadata(parent) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("credential_store_read_failed".to_string()),
        };
        if parent_metadata.file_type().is_symlink()
            || !parent_metadata.is_dir()
            || parent_metadata.mode() & 0o077 != 0
            || parent_metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err("credential_store_permissions_invalid".to_string());
        }

        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("credential_store_read_failed".to_string()),
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err("credential_store_permissions_invalid".to_string());
        }
        if metadata.len() > MAX_CREDENTIAL_BYTES {
            return Err("credential_store_read_failed".to_string());
        }
        Ok(Some(metadata))
    }
}

impl fmt::Debug for FileCredentialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileCredentialStore")
            .field("path", &self.path)
            .field("credential", &"<redacted>")
            .finish()
    }
}

#[cfg(unix)]
impl CredentialStore for FileCredentialStore {
    fn read(&self) -> Result<Option<SecretCredential>, String> {
        if self.validate_private_file()?.is_none() {
            return Ok(None);
        }
        let value = fs::read_to_string(&self.path)
            .map_err(|_| "credential_store_read_failed".to_string())?;
        Ok(Some(SecretCredential::new(value)))
    }

    fn write(&self, credential: &SecretCredential) -> Result<(), String> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        self.ensure_private_directory()?;
        let parent = self.parent()?;
        let temporary = parent.join(format!(".beefapi-managed.{}.tmp", uuid::Uuid::new_v4()));
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|_| "credential_store_write_failed".to_string())?;
            file.write_all(credential.expose().as_bytes())
                .map_err(|_| "credential_store_write_failed".to_string())?;
            file.sync_all()
                .map_err(|_| "credential_store_write_failed".to_string())?;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
                .map_err(|_| "credential_store_write_failed".to_string())?;
            fs::rename(&temporary, &self.path)
                .map_err(|_| "credential_store_write_failed".to_string())?;
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| "credential_store_write_failed".to_string())?;
            self.validate_private_file()?
                .ok_or_else(|| "credential_store_write_failed".to_string())?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }

    fn delete(&self) -> Result<(), String> {
        match fs::symlink_metadata(&self.path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Err("credential_store_permissions_invalid".to_string())
            }
            Ok(_) => fs::remove_file(&self.path)
                .map_err(|_| "credential_store_delete_failed".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("credential_store_delete_failed".to_string()),
        }
    }
}

#[cfg(not(unix))]
impl CredentialStore for FileCredentialStore {
    fn read(&self) -> Result<Option<SecretCredential>, String> {
        Err("credential_store_unavailable".to_string())
    }

    fn write(&self, _credential: &SecretCredential) -> Result<(), String> {
        Err("credential_store_unavailable".to_string())
    }

    fn delete(&self) -> Result<(), String> {
        Err("credential_store_unavailable".to_string())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "beefex-credential-store-test-{}",
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn store(directory: &TestDirectory) -> FileCredentialStore {
        FileCredentialStore::new(directory.0.join("credentials").join("beefapi-managed"))
    }

    #[test]
    fn writes_reads_replaces_and_deletes_with_owner_only_permissions() {
        let directory = TestDirectory::new();
        let store = store(&directory);

        assert!(store.read().unwrap().is_none());
        store
            .write(&SecretCredential::new("first-secret".into()))
            .unwrap();
        assert_eq!(store.read().unwrap().unwrap().expose(), "first-secret");
        store
            .write(&SecretCredential::new("second-secret".into()))
            .unwrap();
        assert_eq!(store.read().unwrap().unwrap().expose(), "second-secret");

        let directory_mode = fs::metadata(store.path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        let file_mode = fs::metadata(&store.path).unwrap().permissions().mode();
        assert_eq!(directory_mode & 0o777, 0o700);
        assert_eq!(file_mode & 0o777, 0o600);
        assert!(fs::read_dir(store.path.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")));

        store.delete().unwrap();
        assert!(store.read().unwrap().is_none());
        store.delete().unwrap();
    }

    #[test]
    fn rejects_broad_permissions_and_symlinks() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let store = store(&directory);
        store
            .write(&SecretCredential::new("private-secret".into()))
            .unwrap();
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            store.read().unwrap_err(),
            "credential_store_permissions_invalid"
        );

        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(
            store.path.parent().unwrap(),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        assert_eq!(
            store.read().unwrap_err(),
            "credential_store_permissions_invalid"
        );
        fs::set_permissions(
            store.path.parent().unwrap(),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();

        fs::remove_file(&store.path).unwrap();
        let outside = directory.0.join("outside");
        fs::write(&outside, "outside-secret").unwrap();
        symlink(&outside, &store.path).unwrap();
        assert_eq!(
            store.read().unwrap_err(),
            "credential_store_permissions_invalid"
        );
        assert_eq!(
            store.delete().unwrap_err(),
            "credential_store_permissions_invalid"
        );
        assert!(outside.exists());
    }

    #[test]
    fn debug_output_redacts_the_credential() {
        let directory = TestDirectory::new();
        let store = store(&directory);
        let secret = SecretCredential::new("never-print-this".into());
        store.write(&secret).unwrap();

        assert_eq!(format!("{secret:?}"), "<redacted>");
        let debug = format!("{store:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("never-print-this"));
    }

    #[test]
    fn write_failure_leaves_no_partial_credential_or_temp_file() {
        let directory = TestDirectory::new();
        let blocked_parent = directory.0.join("blocked-parent");
        fs::write(&blocked_parent, "not-a-directory").unwrap();
        let store = FileCredentialStore::new(blocked_parent.join("beefapi-managed"));

        let result = store.write(&SecretCredential::new("replacement-secret".into()));

        assert_eq!(result.unwrap_err(), "credential_store_write_failed");
        assert!(!store.path.exists());
        assert_eq!(
            fs::read_to_string(&blocked_parent).unwrap(),
            "not-a-directory"
        );
    }
}

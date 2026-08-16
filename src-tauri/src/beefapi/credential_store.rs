use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_CREDENTIAL_BYTES: u64 = 64 * 1024;

fn read_bounded_utf8(path: &Path, error: &str) -> Result<String, String> {
    let file = File::open(path).map_err(|_| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_CREDENTIAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error.to_string())?;
    if bytes.len() as u64 > MAX_CREDENTIAL_BYTES {
        return Err(error.to_string());
    }
    String::from_utf8(bytes).map_err(|_| error.to_string())
}

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

    #[cfg(windows)]
    fn ensure_private_directory(&self) -> Result<(), String> {
        let parent = self.parent()?;
        fs::create_dir_all(parent).map_err(|_| "credential_store_write_failed".to_string())?;
        validate_windows_path_kind(parent, true, "credential_store_permissions_invalid")?;
        apply_windows_private_acl(parent)
            .map_err(|_| "credential_store_write_failed".to_string())?;
        validate_windows_private_acl(parent)
            .map_err(|_| "credential_store_permissions_invalid".to_string())
    }

    #[cfg(windows)]
    fn validate_private_file(&self) -> Result<Option<fs::Metadata>, String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "credential_store_read_failed".to_string())?;
        match fs::symlink_metadata(parent) {
            Ok(_) => {
                validate_windows_path_kind(parent, true, "credential_store_permissions_invalid")?;
                validate_windows_private_acl(parent)
                    .map_err(|_| "credential_store_permissions_invalid".to_string())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("credential_store_read_failed".to_string()),
        }

        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("credential_store_read_failed".to_string()),
        };
        validate_windows_path_kind(&self.path, false, "credential_store_permissions_invalid")?;
        validate_windows_private_acl(&self.path)
            .map_err(|_| "credential_store_permissions_invalid".to_string())?;
        if metadata.len() > MAX_CREDENTIAL_BYTES {
            return Err("credential_store_read_failed".to_string());
        }
        Ok(Some(metadata))
    }
}

#[cfg(windows)]
fn windows_wide(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn validate_windows_path_kind(path: &Path, directory: bool, error: &str) -> Result<(), String> {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    let metadata = fs::symlink_metadata(path).map_err(|_| error.to_string())?;
    let kind_matches = if directory {
        metadata.is_dir()
    } else {
        metadata.is_file()
    };
    if !kind_matches
        || metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
    {
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(windows)]
fn current_windows_user_sid_string() -> Result<String, String> {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)
            .map_err(|_| "credential_store_permissions_invalid".to_string())?;

        let result = (|| {
            let mut required = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut required);
            if required == 0 {
                return Err("credential_store_permissions_invalid".to_string());
            }
            let words = (required as usize).div_ceil(std::mem::size_of::<usize>());
            let mut buffer = vec![0usize; words];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast()),
                required,
                &mut required,
            )
            .map_err(|_| "credential_store_permissions_invalid".to_string())?;
            let token_user = &*(buffer.as_ptr().cast::<TOKEN_USER>());
            windows_sid_string(token_user.User.Sid)
        })();

        let _ = CloseHandle(token);
        result
    }
}

#[cfg(windows)]
fn windows_sid_string(sid: windows::Win32::Security::PSID) -> Result<String, String> {
    use windows::core::PWSTR;
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Authorization::ConvertSidToStringSidW,
    };

    unsafe {
        let mut sid_text = PWSTR::null();
        ConvertSidToStringSidW(sid, &mut sid_text)
            .map_err(|_| "credential_store_permissions_invalid".to_string())?;
        let result = sid_text
            .to_string()
            .map_err(|_| "credential_store_permissions_invalid".to_string());
        let _ = LocalFree(Some(HLOCAL(sid_text.0.cast())));
        result
    }
}

#[cfg(windows)]
fn expected_windows_private_acl() -> Result<
    (
        windows::Win32::Security::PSECURITY_DESCRIPTOR,
        *mut windows::Win32::Security::ACL,
    ),
    String,
> {
    use windows::core::PCWSTR;
    use windows::Win32::Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        GetSecurityDescriptorDacl, ACL, PSECURITY_DESCRIPTOR,
    };

    let sid = current_windows_user_sid_string()?;
    let sddl = windows_wide(std::ffi::OsStr::new(&format!(
        "D:P(A;;FA;;;{sid})(A;;FA;;;SY)"
    )));
    unsafe {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
        .map_err(|_| "credential_store_permissions_invalid".to_string())?;
        let mut present = false.into();
        let mut defaulted = false.into();
        let mut acl: *mut ACL = std::ptr::null_mut();
        if GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted).is_err()
            || !present.as_bool()
            || acl.is_null()
        {
            use windows::Win32::Foundation::{LocalFree, HLOCAL};
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            return Err("credential_store_permissions_invalid".to_string());
        }
        Ok((descriptor, acl))
    }
}

#[cfg(windows)]
fn acl_bytes(acl: *const windows::Win32::Security::ACL) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::{AclSizeInformation, GetAclInformation, ACL_SIZE_INFORMATION};

    unsafe {
        let mut info = ACL_SIZE_INFORMATION::default();
        GetAclInformation(
            acl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
        .map_err(|_| "credential_store_permissions_invalid".to_string())?;
        if info.AclBytesInUse == 0 {
            return Err("credential_store_permissions_invalid".to_string());
        }
        Ok(std::slice::from_raw_parts(acl.cast::<u8>(), info.AclBytesInUse as usize).to_vec())
    }
}

#[cfg(windows)]
fn apply_windows_private_acl(path: &Path) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::{
            Authorization::{ConvertStringSidToSidW, SetNamedSecurityInfoW, SE_FILE_OBJECT},
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, PSID,
        },
    };

    let wide = windows_wide(path.as_os_str());
    let owner_sid_text = current_windows_user_sid_string()?;
    let owner_sid_text = windows_wide(std::ffi::OsStr::new(&owner_sid_text));
    let mut owner_sid = PSID::default();
    unsafe {
        ConvertStringSidToSidW(PCWSTR(owner_sid_text.as_ptr()), &mut owner_sid)
            .map_err(|_| "credential_store_permissions_invalid".to_string())?;
    }
    let (descriptor, acl) = match expected_windows_private_acl() {
        Ok(value) => value,
        Err(error) => {
            unsafe {
                let _ = LocalFree(Some(HLOCAL(owner_sid.0)));
            }
            return Err(error);
        }
    };
    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION
                | OWNER_SECURITY_INFORMATION
                | PROTECTED_DACL_SECURITY_INFORMATION,
            Some(owner_sid),
            None,
            Some(acl),
            None,
        )
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(owner_sid.0)));
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
    }
    status
        .ok()
        .map_err(|_| "credential_store_permissions_invalid".to_string())
}

#[cfg(windows)]
fn validate_windows_private_acl(path: &Path) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::{
            Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT},
            ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        },
    };

    let wide = windows_wide(path.as_os_str());
    let mut actual_owner = PSID::default();
    let mut actual_acl: *mut ACL = std::ptr::null_mut();
    let mut actual_descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            Some(&mut actual_owner),
            None,
            Some(&mut actual_acl),
            None,
            &mut actual_descriptor,
        )
    };
    status
        .ok()
        .map_err(|_| "credential_store_permissions_invalid".to_string())?;
    if actual_owner.0.is_null() || actual_acl.is_null() {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(actual_descriptor.0)));
        }
        return Err("credential_store_permissions_invalid".to_string());
    }

    let actual_owner = windows_sid_string(actual_owner);
    let expected_owner = current_windows_user_sid_string();
    let (expected_descriptor, expected_acl) = expected_windows_private_acl()?;
    let actual = acl_bytes(actual_acl);
    let expected = acl_bytes(expected_acl);
    unsafe {
        let _ = LocalFree(Some(HLOCAL(actual_descriptor.0)));
        let _ = LocalFree(Some(HLOCAL(expected_descriptor.0)));
    }
    if actual_owner? != expected_owner? || actual? != expected? {
        return Err("credential_store_permissions_invalid".to_string());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn atomic_replace_windows(source: &Path, destination: &Path) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH, REPLACEFILE_WRITE_THROUGH,
    };

    let destination_exists = destination.exists();
    let source = windows_wide(source.as_os_str());
    let destination = windows_wide(destination.as_os_str());
    unsafe {
        if destination_exists {
            ReplaceFileW(
                PCWSTR(destination.as_ptr()),
                PCWSTR(source.as_ptr()),
                PCWSTR::null(),
                REPLACEFILE_WRITE_THROUGH,
                None,
                None,
            )
        } else {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
        }
    }
    .map_err(|_| "credential_store_write_failed".to_string())
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
        let value = read_bounded_utf8(&self.path, "credential_store_read_failed")?;
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

#[cfg(windows)]
impl CredentialStore for FileCredentialStore {
    fn read(&self) -> Result<Option<SecretCredential>, String> {
        if self.validate_private_file()?.is_none() {
            return Ok(None);
        }
        let value = read_bounded_utf8(&self.path, "credential_store_read_failed")?;
        Ok(Some(SecretCredential::new(value)))
    }

    fn write(&self, credential: &SecretCredential) -> Result<(), String> {
        self.ensure_private_directory()?;
        let parent = self.parent()?;
        let temporary = parent.join(format!(".beefapi-managed.{}.tmp", uuid::Uuid::new_v4()));
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|_| "credential_store_write_failed".to_string())?;
            file.write_all(credential.expose().as_bytes())
                .map_err(|_| "credential_store_write_failed".to_string())?;
            file.sync_all()
                .map_err(|_| "credential_store_write_failed".to_string())?;
            drop(file);
            apply_windows_private_acl(&temporary)
                .map_err(|_| "credential_store_write_failed".to_string())?;
            validate_windows_private_acl(&temporary)
                .map_err(|_| "credential_store_write_failed".to_string())?;
            atomic_replace_windows(&temporary, &self.path)?;
            apply_windows_private_acl(&self.path)
                .map_err(|_| "credential_store_write_failed".to_string())?;
            self.validate_private_file()?
                .ok_or_else(|| "credential_store_write_failed".to_string())?;
            let readback = read_bounded_utf8(&self.path, "credential_store_write_failed")?;
            if readback.as_bytes() != credential.expose().as_bytes() {
                return Err("credential_store_write_failed".to_string());
            }
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }

    fn delete(&self) -> Result<(), String> {
        if self.validate_private_file()?.is_none() {
            return Ok(());
        }
        fs::remove_file(&self.path).map_err(|_| "credential_store_delete_failed".to_string())
    }
}

#[cfg(all(not(unix), not(windows)))]
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    fn apply_test_acl(path: &Path, sddl: &str) {
        use windows::core::PCWSTR;
        use windows::Win32::{
            Foundation::{LocalFree, HLOCAL},
            Security::{
                Authorization::{
                    ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
                    SDDL_REVISION_1, SE_FILE_OBJECT,
                },
                GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION,
                PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
            },
        };

        let wide_sddl = windows_wide(std::ffi::OsStr::new(sddl));
        let wide_path = windows_wide(path.as_os_str());
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide_sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .unwrap();
            let mut present = false.into();
            let mut defaulted = false.into();
            let mut acl: *mut ACL = std::ptr::null_mut();
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted).unwrap();
            assert!(present.as_bool());
            assert!(!acl.is_null());
            SetNamedSecurityInfoW(
                PCWSTR(wide_path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(acl),
                None,
            )
            .ok()
            .unwrap();
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "beefex-windows-credential-store-test-{}",
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
    fn writes_reads_replaces_and_deletes_private_credential() {
        let directory = TestDirectory::new();
        let store = store(&directory);

        assert!(store.read().unwrap().is_none());
        store
            .write(&SecretCredential::new("first-windows-secret".into()))
            .unwrap();
        assert_eq!(
            store.read().unwrap().unwrap().expose(),
            "first-windows-secret"
        );
        store
            .write(&SecretCredential::new("second-windows-secret".into()))
            .unwrap();
        assert_eq!(
            store.read().unwrap().unwrap().expose(),
            "second-windows-secret"
        );
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
    fn write_failure_leaves_no_partial_credential() {
        let directory = TestDirectory::new();
        let blocked_parent = directory.0.join("blocked-parent");
        fs::write(&blocked_parent, "not-a-directory").unwrap();
        let store = FileCredentialStore::new(blocked_parent.join("beefapi-managed"));

        assert_eq!(
            store
                .write(&SecretCredential::new("replacement-secret".into()))
                .unwrap_err(),
            "credential_store_write_failed"
        );
        assert!(!store.path.exists());
        assert_eq!(
            fs::read_to_string(&blocked_parent).unwrap(),
            "not-a-directory"
        );
    }

    #[test]
    fn debug_output_redacts_windows_credential() {
        let directory = TestDirectory::new();
        let store = store(&directory);
        let secret = SecretCredential::new("never-print-windows-secret".into());

        assert_eq!(format!("{secret:?}"), "<redacted>");
        let debug = format!("{store:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("never-print-windows-secret"));
    }

    #[test]
    fn rejects_oversized_private_credential() {
        let directory = TestDirectory::new();
        let store = store(&directory);
        store
            .write(&SecretCredential::new("small-secret".into()))
            .unwrap();

        fs::write(&store.path, vec![b'x'; MAX_CREDENTIAL_BYTES as usize + 1]).unwrap();

        assert_eq!(store.read().unwrap_err(), "credential_store_read_failed");
    }

    #[test]
    fn rejects_broad_windows_acl() {
        let directory = TestDirectory::new();
        let store = store(&directory);
        store
            .write(&SecretCredential::new("private-secret".into()))
            .unwrap();
        apply_test_acl(&store.path, "D:P(A;;FA;;;WD)");

        assert_eq!(
            store.read().unwrap_err(),
            "credential_store_permissions_invalid"
        );
        assert_eq!(
            store.delete().unwrap_err(),
            "credential_store_permissions_invalid"
        );
    }

    #[test]
    fn rejects_windows_reparse_file_without_touching_target() {
        use std::os::windows::fs::symlink_file;

        let directory = TestDirectory::new();
        let store = store(&directory);
        store
            .write(&SecretCredential::new("private-secret".into()))
            .unwrap();
        fs::remove_file(&store.path).unwrap();
        let outside = directory.0.join("outside-secret");
        fs::write(&outside, "outside-secret").unwrap();
        symlink_file(&outside, &store.path).unwrap();

        assert_eq!(
            store.read().unwrap_err(),
            "credential_store_permissions_invalid"
        );
        assert_eq!(
            store.delete().unwrap_err(),
            "credential_store_permissions_invalid"
        );
        assert_eq!(fs::read_to_string(outside).unwrap(), "outside-secret");
    }
}

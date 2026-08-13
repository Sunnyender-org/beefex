use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::Duration,
};

use tauri::{AppHandle, Emitter, State};
use tauri_plugin_shell::ShellExt;

use super::{
    credential_store::{CredentialStore, SecretCredential},
    provider::{
        hydrate_managed_provider, validate_discovery, validate_managed_credential,
        EphemeralModelProvider,
    },
    types::{
        AccountPhase, AccountState, AuthStartResponse, DiscoveryResponse, ManagedCredential,
        PollResponse, SafeAccountMetadata,
    },
};

pub(crate) type TransportFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

#[cfg(not(test))]
const CREDENTIAL_STORE_READ_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(test)]
const CREDENTIAL_STORE_READ_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) trait AccountTransport: Send + Sync {
    fn trusted_base_url(&self) -> &str;
    fn start<'a>(
        &'a self,
        client_version: &'a str,
        hostname: &'a str,
    ) -> TransportFuture<'a, AuthStartResponse>;
    fn poll<'a>(&'a self, device_code: &'a str) -> TransportFuture<'a, PollResponse>;
    fn discover<'a>(
        &'a self,
        credential: &'a SecretCredential,
        base_url: &'a str,
    ) -> TransportFuture<'a, DiscoveryResponse>;
    fn revoke<'a>(
        &'a self,
        credential: &'a SecretCredential,
        base_url: &'a str,
    ) -> TransportFuture<'a, ()>;
}

pub(crate) trait AccountMetadataStore: Send + Sync {
    fn load(&self) -> Result<Option<SafeAccountMetadata>, String>;
    fn save(&self, metadata: &SafeAccountMetadata) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

pub(crate) struct FileAccountMetadataStore {
    path: PathBuf,
}

impl FileAccountMetadataStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl AccountMetadataStore for FileAccountMetadataStore {
    fn load(&self) -> Result<Option<SafeAccountMetadata>, String> {
        match std::fs::read(&self.path) {
            Ok(content) => serde_json::from_slice(&content)
                .map(Some)
                .map_err(|_| "account_metadata_invalid".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("account_metadata_read_failed".to_string()),
        }
    }

    fn save(&self, metadata: &SafeAccountMetadata) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "account_metadata_write_failed".to_string())?;
        std::fs::create_dir_all(parent).map_err(|_| "account_metadata_write_failed".to_string())?;
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(metadata)
                .map_err(|_| "account_metadata_write_failed".to_string())?,
        )
        .map_err(|_| "account_metadata_write_failed".to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "account_metadata_write_failed".to_string())?;
        }
        std::fs::rename(temporary, &self.path)
            .map_err(|_| "account_metadata_write_failed".to_string())
    }

    fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("account_metadata_delete_failed".to_string()),
        }
    }
}

pub(crate) struct PendingAuthorization {
    generation: u64,
    response: AuthStartResponse,
}

pub(crate) struct AccountService {
    transport: Arc<dyn AccountTransport>,
    credentials: Arc<dyn CredentialStore>,
    metadata: Arc<dyn AccountMetadataStore>,
    state: RwLock<AccountState>,
    auth_generation: AtomicU64,
}

impl AccountService {
    pub(crate) fn production(metadata_path: PathBuf) -> Result<Self, String> {
        let (transport, credentials, metadata) = super::auth::production_parts(metadata_path)?;
        Ok(Self::from_parts(transport, credentials, metadata))
    }

    pub(crate) fn unavailable() -> Self {
        Self::from_parts(
            Arc::new(UnavailableAccountBackend),
            Arc::new(UnavailableAccountBackend),
            Arc::new(UnavailableAccountBackend),
        )
    }

    pub(crate) fn from_parts(
        transport: Arc<dyn AccountTransport>,
        credentials: Arc<dyn CredentialStore>,
        metadata: Arc<dyn AccountMetadataStore>,
    ) -> Self {
        Self {
            transport,
            credentials,
            metadata,
            state: RwLock::new(AccountState::signed_out(Some("initializing"))),
            auth_generation: AtomicU64::new(0),
        }
    }

    pub(crate) fn state(&self) -> AccountState {
        self.state
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) async fn reconcile(&self, on_state: impl Fn(AccountState)) -> AccountState {
        let metadata = match self.metadata_load().await {
            Ok(metadata) => metadata,
            Err(reason) => {
                return self.publish(
                    AccountState::failure(AccountPhase::CleanupRequired, &reason),
                    &on_state,
                );
            }
        };
        let credential = match self.credential_read().await {
            Ok(credential) => credential,
            Err(reason) => {
                return self.publish(
                    AccountState::failure(AccountPhase::CredentialStoreFailed, &reason),
                    &on_state,
                );
            }
        };
        match (metadata, credential) {
            (None, None) => self.publish(AccountState::signed_out(None), &on_state),
            (Some(_), None) => {
                let _ = self.metadata_clear().await;
                self.publish(
                    AccountState::signed_out(Some("reauthorization_required")),
                    &on_state,
                )
            }
            (None, Some(credential)) => {
                let cleanup = ManagedCredential {
                    credential,
                    base_url: self.transport.trusted_base_url().to_string(),
                    user_email: String::new(),
                    key_name: String::new(),
                    group: super::types::REQUIRED_GROUP.to_string(),
                };
                self.cleanup_issued_credential(cleanup, "account_metadata_missing", &on_state)
                    .await
            }
            (Some(metadata), Some(credential)) => {
                self.reconcile_pair(metadata, credential, &on_state).await
            }
        }
    }

    pub(crate) async fn resolve_managed_provider(
        &self,
        requested_model: Option<&str>,
        on_state: impl Fn(AccountState),
    ) -> Result<EphemeralModelProvider, String> {
        if !matches!(
            self.state().phase,
            AccountPhase::SignedIn | AccountPhase::Offline
        ) {
            return Err("reauthorization_required".to_string());
        }
        let metadata = self
            .metadata_load()
            .await?
            .ok_or_else(|| "reauthorization_required".to_string())?;
        let credential = self
            .credential_read()
            .await?
            .ok_or_else(|| "reauthorization_required".to_string())?;
        let managed = ManagedCredential {
            credential,
            base_url: metadata.base_url.clone(),
            user_email: metadata.email.clone(),
            key_name: metadata.key_name.clone(),
            group: metadata.group.clone(),
        };
        if validate_managed_credential(&managed).is_err()
            || managed.base_url != self.transport.trusted_base_url()
        {
            self.cleanup_issued_credential(managed, "untrusted_account_metadata", &on_state)
                .await;
            return Err("reauthorization_required".to_string());
        }
        let discovery = match self
            .transport
            .discover(&managed.credential, &managed.base_url)
            .await
        {
            Ok(discovery) => discovery,
            Err(reason) if reason == "network_unavailable" => {
                self.publish(
                    AccountState::from_metadata(
                        &metadata,
                        AccountPhase::Offline,
                        Some("network_unavailable"),
                    ),
                    &on_state,
                );
                return Err(reason);
            }
            Err(reason) => {
                self.cleanup_issued_credential(managed, &reason, &on_state)
                    .await;
                return Err("reauthorization_required".to_string());
            }
        };
        let validated = match validate_discovery(&managed, &discovery) {
            Ok(validated) => validated,
            Err(reason) => {
                self.cleanup_issued_credential(managed, reason, &on_state)
                    .await;
                return Err("reauthorization_required".to_string());
            }
        };
        self.publish(AccountState::signed_in(&validated), &on_state);
        hydrate_managed_provider(&validated, &managed.credential, requested_model)
            .map_err(str::to_string)
    }

    pub(crate) async fn logout(&self, on_state: impl Fn(AccountState)) -> AccountState {
        self.auth_generation.fetch_add(1, Ordering::SeqCst);
        let metadata = self.metadata_load().await.ok().flatten();
        let credential = self.credential_read().await.ok().flatten();
        let revoke_failed = if let Some(credential) = credential.as_ref() {
            self.transport
                .revoke(
                    credential,
                    metadata
                        .as_ref()
                        .map(|metadata| metadata.base_url.as_str())
                        .unwrap_or_else(|| self.transport.trusted_base_url()),
                )
                .await
                .is_err()
        } else {
            false
        };
        let delete_failed = self.credential_delete().await.is_err();
        let metadata_failed = self.metadata_clear().await.is_err();
        if revoke_failed || delete_failed || metadata_failed {
            return self.publish(
                AccountState::failure(AccountPhase::CleanupRequired, "cleanup_required"),
                &on_state,
            );
        }
        self.publish(AccountState::signed_out(Some("logged_out")), &on_state)
    }

    pub(crate) async fn handle_inference_credential_rejected(
        &self,
        on_state: impl Fn(AccountState),
    ) -> AccountState {
        self.auth_generation.fetch_add(1, Ordering::SeqCst);
        let metadata = self.metadata_load().await.ok().flatten();
        let credential = self.credential_read().await.ok().flatten();
        if let Some(credential) = credential {
            let managed = ManagedCredential {
                credential,
                base_url: metadata
                    .as_ref()
                    .map(|metadata| metadata.base_url.clone())
                    .unwrap_or_else(|| self.transport.trusted_base_url().to_string()),
                user_email: metadata
                    .as_ref()
                    .map(|metadata| metadata.email.clone())
                    .unwrap_or_default(),
                key_name: metadata
                    .as_ref()
                    .map(|metadata| metadata.key_name.clone())
                    .unwrap_or_default(),
                group: metadata
                    .as_ref()
                    .map(|metadata| metadata.group.clone())
                    .unwrap_or_else(|| super::types::REQUIRED_GROUP.to_string()),
            };
            return self
                .cleanup_issued_credential(managed, "reauthorization_required", &on_state)
                .await;
        }
        let credential_delete_failed = self.credential_delete().await.is_err();
        let metadata_clear_failed = self.metadata_clear().await.is_err();
        let cleanup_failed = credential_delete_failed || metadata_clear_failed;
        if cleanup_failed {
            return self.publish(
                AccountState::failure(AccountPhase::CleanupRequired, "cleanup_required"),
                &on_state,
            );
        }
        self.publish(
            AccountState::signed_out(Some("reauthorization_required")),
            &on_state,
        )
    }

    pub(crate) async fn begin_authorization(
        &self,
        client_version: &str,
        hostname: &str,
    ) -> Result<PendingAuthorization, String> {
        let generation = self.auth_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let response = match self.transport.start(client_version, hostname).await {
            Ok(response) => response,
            Err(reason) => {
                let phase = if reason == "network_unavailable" {
                    AccountPhase::Offline
                } else {
                    AccountPhase::SignedOut
                };
                self.replace_state(AccountState::failure(phase, &reason));
                return Err(reason);
            }
        };
        if response.device_code.trim().is_empty()
            || response.user_code.trim().is_empty()
            || response.expires_in == 0
        {
            return Err("invalid_authorization_response".to_string());
        }
        let state = AccountState {
            phase: AccountPhase::Authorizing,
            email: None,
            group: None,
            default_model: None,
            allowed_models: Vec::new(),
            key_name: None,
            user_code: Some(response.user_code.clone()),
            verification_uri: Some(response.verification_uri.clone()),
            verification_uri_complete: Some(response.verification_uri_complete.clone()),
            expires_at: Some(
                chrono::Utc::now().timestamp()
                    + i64::try_from(response.expires_in).unwrap_or(i64::MAX),
            ),
            reason: None,
        };
        self.replace_state(state);
        Ok(PendingAuthorization {
            generation,
            response,
        })
    }

    pub(crate) async fn complete_authorization(
        &self,
        authorization: PendingAuthorization,
        on_state: impl Fn(AccountState),
    ) -> Result<AccountState, String> {
        let deadline =
            tokio::time::Instant::now() + Duration::from_secs(authorization.response.expires_in);
        let mut interval = Duration::from_secs(authorization.response.interval.max(1));
        self.transition(AccountPhase::Polling, None, &on_state);
        loop {
            tokio::time::sleep(interval).await;
            if authorization.generation != self.auth_generation.load(Ordering::SeqCst) {
                return Ok(self.state());
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(self.transition(
                    AccountPhase::Expired,
                    Some("authorization_expired"),
                    &on_state,
                ));
            }
            match self
                .transport
                .poll(&authorization.response.device_code)
                .await
            {
                Ok(PollResponse::Pending) => {
                    self.transition(AccountPhase::Polling, None, &on_state);
                }
                Ok(PollResponse::SlowDown) => {
                    interval = interval.saturating_add(Duration::from_secs(5));
                    self.transition(AccountPhase::Polling, Some("slow_down"), &on_state);
                }
                Ok(PollResponse::Approved(credential)) => {
                    if authorization.generation != self.auth_generation.load(Ordering::SeqCst) {
                        let _ = self
                            .transport
                            .revoke(&credential.credential, self.transport.trusted_base_url())
                            .await;
                        return Ok(self.state());
                    }
                    return self.finish_approved(credential, &on_state).await;
                }
                Ok(PollResponse::Denied) => {
                    return Ok(self.transition(
                        AccountPhase::Denied,
                        Some("authorization_denied"),
                        &on_state,
                    ));
                }
                Ok(PollResponse::Expired) => {
                    return Ok(self.transition(
                        AccountPhase::Expired,
                        Some("authorization_expired"),
                        &on_state,
                    ));
                }
                Ok(PollResponse::EntitlementMissing) => {
                    return Ok(self.transition(
                        AccountPhase::EntitlementMissing,
                        Some("entitlement_required"),
                        &on_state,
                    ));
                }
                Ok(PollResponse::DefaultModelUnavailable) => {
                    return Ok(self.transition(
                        AccountPhase::SignedOut,
                        Some("default_model_unavailable"),
                        &on_state,
                    ));
                }
                Err(_) => {
                    interval = interval.saturating_mul(2).min(Duration::from_secs(60));
                    self.transition(
                        AccountPhase::Offline,
                        Some("network_unavailable"),
                        &on_state,
                    );
                }
            }
        }
    }

    pub(crate) fn cancel_authorization(&self) -> AccountState {
        self.auth_generation.fetch_add(1, Ordering::SeqCst);
        let state = AccountState::signed_out(Some("authorization_cancelled"));
        self.replace_state(state.clone());
        state
    }

    async fn finish_approved(
        &self,
        credential: ManagedCredential,
        on_state: &impl Fn(AccountState),
    ) -> Result<AccountState, String> {
        if credential.base_url != self.transport.trusted_base_url() {
            return Ok(self
                .cleanup_issued_credential(credential, "untrusted_base_url", on_state)
                .await);
        }
        if let Err(reason) = validate_managed_credential(&credential) {
            return Ok(self
                .cleanup_issued_credential(credential, reason, on_state)
                .await);
        }
        let discovery = match self
            .transport
            .discover(&credential.credential, &credential.base_url)
            .await
        {
            Ok(discovery) => discovery,
            Err(reason) => {
                return Ok(self
                    .cleanup_issued_credential(credential, &reason, on_state)
                    .await);
            }
        };
        let metadata = match validate_discovery(&credential, &discovery) {
            Ok(metadata) => metadata,
            Err(reason) => {
                return Ok(self
                    .cleanup_issued_credential(credential, reason, on_state)
                    .await);
            }
        };
        if self
            .credential_write_verified(credential.credential.expose())
            .await
            .is_err()
        {
            return Ok(self
                .cleanup_issued_credential(credential, "credential_store_failed", on_state)
                .await);
        }
        if self.metadata_save(metadata.clone()).await.is_err() {
            let _ = self.credential_delete().await;
            return Ok(self
                .cleanup_issued_credential(credential, "account_metadata_write_failed", on_state)
                .await);
        }
        let state = AccountState::signed_in(&metadata);
        self.replace_state(state.clone());
        on_state(state.clone());
        Ok(state)
    }

    async fn cleanup_issued_credential(
        &self,
        credential: ManagedCredential,
        reason: &str,
        on_state: &impl Fn(AccountState),
    ) -> AccountState {
        let credential_delete_failed = self.credential_delete().await.is_err();
        let metadata_clear_failed = self.metadata_clear().await.is_err();
        let revoke_base_url = if validate_managed_credential(&credential).is_ok()
            && credential.base_url == self.transport.trusted_base_url()
        {
            credential.base_url.as_str()
        } else {
            self.transport.trusted_base_url()
        };
        let revoke_failed = self
            .transport
            .revoke(&credential.credential, revoke_base_url)
            .await
            .is_err();
        let state = if credential_delete_failed || metadata_clear_failed || revoke_failed {
            AccountState::failure(AccountPhase::CleanupRequired, "cleanup_required")
        } else {
            let phase = if reason == "credential_store_failed" {
                AccountPhase::CredentialStoreFailed
            } else {
                AccountPhase::SignedOut
            };
            AccountState::failure(phase, reason)
        };
        self.replace_state(state.clone());
        on_state(state.clone());
        state
    }

    async fn reconcile_pair(
        &self,
        metadata: SafeAccountMetadata,
        credential: SecretCredential,
        on_state: &impl Fn(AccountState),
    ) -> AccountState {
        let managed = ManagedCredential {
            credential,
            base_url: metadata.base_url.clone(),
            user_email: metadata.email.clone(),
            key_name: metadata.key_name.clone(),
            group: metadata.group.clone(),
        };
        if validate_managed_credential(&managed).is_err()
            || managed.base_url != self.transport.trusted_base_url()
        {
            return self
                .cleanup_issued_credential(managed, "untrusted_account_metadata", on_state)
                .await;
        }
        match self
            .transport
            .discover(&managed.credential, &managed.base_url)
            .await
        {
            Ok(discovery) => match validate_discovery(&managed, &discovery) {
                Ok(validated) => self.publish(AccountState::signed_in(&validated), on_state),
                Err(reason) => {
                    self.cleanup_issued_credential(managed, reason, on_state)
                        .await
                }
            },
            Err(reason) if reason == "network_unavailable" => self.publish(
                AccountState::from_metadata(
                    &metadata,
                    AccountPhase::Offline,
                    Some("network_unavailable"),
                ),
                on_state,
            ),
            Err(reason) => {
                self.cleanup_issued_credential(managed, &reason, on_state)
                    .await
            }
        }
    }

    async fn credential_read(&self) -> Result<Option<SecretCredential>, String> {
        let credentials = self.credentials.clone();
        let read = tokio::task::spawn_blocking(move || credentials.read());
        tokio::time::timeout(CREDENTIAL_STORE_READ_TIMEOUT, read)
            .await
            .map_err(|_| "credential_store_read_timeout".to_string())?
            .map_err(|_| "credential_store_read_failed".to_string())?
    }

    async fn credential_write_verified(&self, value: &str) -> Result<(), String> {
        let credentials = self.credentials.clone();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            credentials.write(&SecretCredential::new(value.clone()))?;
            let stored = credentials
                .read()?
                .ok_or_else(|| "credential_store_write_failed".to_string())?;
            if stored.expose() != value {
                return Err("credential_store_write_failed".to_string());
            }
            Ok(())
        })
        .await
        .map_err(|_| "credential_store_write_failed".to_string())?
    }

    async fn credential_delete(&self) -> Result<(), String> {
        let credentials = self.credentials.clone();
        tokio::task::spawn_blocking(move || credentials.delete())
            .await
            .map_err(|_| "credential_store_delete_failed".to_string())?
    }

    async fn metadata_load(&self) -> Result<Option<SafeAccountMetadata>, String> {
        let metadata = self.metadata.clone();
        tokio::task::spawn_blocking(move || metadata.load())
            .await
            .map_err(|_| "account_metadata_read_failed".to_string())?
    }

    async fn metadata_save(&self, value: SafeAccountMetadata) -> Result<(), String> {
        let metadata = self.metadata.clone();
        tokio::task::spawn_blocking(move || metadata.save(&value))
            .await
            .map_err(|_| "account_metadata_write_failed".to_string())?
    }

    async fn metadata_clear(&self) -> Result<(), String> {
        let metadata = self.metadata.clone();
        tokio::task::spawn_blocking(move || metadata.clear())
            .await
            .map_err(|_| "account_metadata_delete_failed".to_string())?
    }

    fn transition(
        &self,
        phase: AccountPhase,
        reason: Option<&str>,
        on_state: &impl Fn(AccountState),
    ) -> AccountState {
        let current = self.state();
        let state = AccountState {
            phase,
            reason: reason.map(str::to_string),
            ..current
        };
        self.replace_state(state.clone());
        on_state(state.clone());
        state
    }

    fn replace_state(&self, state: AccountState) {
        *self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner()) = state;
    }

    fn publish(&self, state: AccountState, on_state: &impl Fn(AccountState)) -> AccountState {
        self.replace_state(state.clone());
        on_state(state.clone());
        state
    }
}

struct UnavailableAccountBackend;

impl AccountTransport for UnavailableAccountBackend {
    fn trusted_base_url(&self) -> &str {
        super::types::PRODUCTION_BASE_URL
    }

    fn start<'a>(
        &'a self,
        _client_version: &'a str,
        _hostname: &'a str,
    ) -> TransportFuture<'a, AuthStartResponse> {
        Box::pin(async { Err("beefapi_account_unavailable".to_string()) })
    }

    fn poll<'a>(&'a self, _device_code: &'a str) -> TransportFuture<'a, PollResponse> {
        Box::pin(async { Err("beefapi_account_unavailable".to_string()) })
    }

    fn discover<'a>(
        &'a self,
        _credential: &'a SecretCredential,
        _base_url: &'a str,
    ) -> TransportFuture<'a, DiscoveryResponse> {
        Box::pin(async { Err("beefapi_account_unavailable".to_string()) })
    }

    fn revoke<'a>(
        &'a self,
        _credential: &'a SecretCredential,
        _base_url: &'a str,
    ) -> TransportFuture<'a, ()> {
        Box::pin(async { Err("beefapi_account_unavailable".to_string()) })
    }
}

impl CredentialStore for UnavailableAccountBackend {
    fn read(&self) -> Result<Option<SecretCredential>, String> {
        Ok(None)
    }

    fn write(&self, _credential: &SecretCredential) -> Result<(), String> {
        Err("credential_store_unavailable".to_string())
    }

    fn delete(&self) -> Result<(), String> {
        Ok(())
    }
}

impl AccountMetadataStore for UnavailableAccountBackend {
    fn load(&self) -> Result<Option<SafeAccountMetadata>, String> {
        Ok(None)
    }

    fn save(&self, _metadata: &SafeAccountMetadata) -> Result<(), String> {
        Err("account_metadata_unavailable".to_string())
    }

    fn clear(&self) -> Result<(), String> {
        Ok(())
    }
}

#[tauri::command]
pub(crate) fn beefapi_account_state(state: State<'_, crate::state::AppState>) -> AccountState {
    state.beefapi_account.state()
}

#[tauri::command]
pub(crate) async fn beefapi_account_reconnect(
    app: AppHandle,
    state: State<'_, crate::state::AppState>,
) -> Result<AccountState, String> {
    let account = state.beefapi_account.clone();
    drop(state);
    let event_app = app.clone();
    Ok(account
        .reconcile(move |state| emit_account_state(&event_app, &state))
        .await)
}

#[tauri::command]
#[allow(deprecated)]
pub(crate) async fn beefapi_auth_start(
    app: AppHandle,
    state: State<'_, crate::state::AppState>,
) -> Result<AccountState, String> {
    let account = state.beefapi_account.clone();
    let authorization = match account
        .begin_authorization(
            &app.package_info().version.to_string(),
            &std::env::var("HOSTNAME").unwrap_or_else(|_| "Desktop".to_string()),
        )
        .await
    {
        Ok(authorization) => authorization,
        Err(error) => {
            emit_account_state(&app, &account.state());
            return Err(error);
        }
    };
    let current = account.state();
    emit_account_state(&app, &current);
    if let Some(url) = current.verification_uri_complete.as_deref() {
        let _ = app.shell().open(url, None);
    }
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = account
            .complete_authorization(authorization, move |state| {
                emit_account_state(&event_app, &state);
            })
            .await;
    });
    Ok(current)
}

#[tauri::command]
pub(crate) fn beefapi_auth_cancel(
    app: AppHandle,
    state: State<'_, crate::state::AppState>,
) -> AccountState {
    let account = state.beefapi_account.cancel_authorization();
    emit_account_state(&app, &account);
    account
}

#[tauri::command]
#[allow(deprecated)]
pub(crate) fn beefapi_auth_reopen_browser(
    app: AppHandle,
    state: State<'_, crate::state::AppState>,
) -> Result<(), String> {
    let account = state.beefapi_account.state();
    let url = account
        .verification_uri_complete
        .or(account.verification_uri)
        .ok_or_else(|| "authorization_not_active".to_string())?;
    app.shell()
        .open(url, None)
        .map_err(|_| "browser_open_failed".to_string())
}

#[tauri::command]
pub(crate) async fn beefapi_logout(
    app: AppHandle,
    state: State<'_, crate::state::AppState>,
) -> Result<AccountState, String> {
    let account = state.beefapi_account.clone();
    drop(state);
    let event_app = app.clone();
    Ok(account
        .logout(move |state| emit_account_state(&event_app, &state))
        .await)
}

pub(crate) fn emit_account_state(app: &AppHandle, state: &AccountState) {
    let _ = app.emit("beefapi-account-state", state);
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Condvar, Mutex,
        },
        time::Duration,
    };

    use super::{AccountService, AccountTransport, TransportFuture};
    use crate::beefapi::{
        credential_store::{CredentialStore, SecretCredential},
        types::{
            AccountPhase, AuthStartResponse, DiscoveryResponse, ManagedCredential, PollResponse,
            SafeAccountMetadata,
        },
    };

    struct FakeTransport {
        polls: Mutex<VecDeque<Result<PollResponse, String>>>,
        discovery: Mutex<Option<Result<DiscoveryResponse, String>>>,
        discoveries: AtomicUsize,
        events: Arc<Mutex<Vec<&'static str>>>,
        revokes: AtomicUsize,
        revoke_fails: bool,
    }

    struct OfflineStartTransport;

    impl AccountTransport for OfflineStartTransport {
        fn trusted_base_url(&self) -> &str {
            "https://beefapi.com/v1"
        }

        fn start<'a>(
            &'a self,
            _client_version: &'a str,
            _hostname: &'a str,
        ) -> TransportFuture<'a, AuthStartResponse> {
            Box::pin(async { Err("network_unavailable".to_string()) })
        }

        fn poll<'a>(&'a self, _device_code: &'a str) -> TransportFuture<'a, PollResponse> {
            Box::pin(async { unreachable!() })
        }

        fn discover<'a>(
            &'a self,
            _credential: &'a SecretCredential,
            _base_url: &'a str,
        ) -> TransportFuture<'a, DiscoveryResponse> {
            Box::pin(async { unreachable!() })
        }

        fn revoke<'a>(
            &'a self,
            _credential: &'a SecretCredential,
            _base_url: &'a str,
        ) -> TransportFuture<'a, ()> {
            Box::pin(async { unreachable!() })
        }
    }

    impl AccountTransport for FakeTransport {
        fn trusted_base_url(&self) -> &str {
            "https://beefapi.com/v1"
        }

        fn start<'a>(
            &'a self,
            _client_version: &'a str,
            _hostname: &'a str,
        ) -> TransportFuture<'a, AuthStartResponse> {
            Box::pin(async {
                Ok(AuthStartResponse {
                    device_code: "device-private".to_string(),
                    user_code: "BEEF-CODE".to_string(),
                    verification_uri: "https://beefapi.com/desktop-auth".to_string(),
                    verification_uri_complete: "https://beefapi.com/desktop-auth?code=BEEF-CODE"
                        .to_string(),
                    expires_in: 60,
                    interval: 3,
                })
            })
        }

        fn poll<'a>(&'a self, _device_code: &'a str) -> TransportFuture<'a, PollResponse> {
            Box::pin(async move {
                self.polls
                    .lock()
                    .unwrap()
                    .pop_front()
                    .ok_or_else(|| "missing fake poll".to_string())
                    .and_then(|response| response)
            })
        }

        fn discover<'a>(
            &'a self,
            _credential: &'a SecretCredential,
            _base_url: &'a str,
        ) -> TransportFuture<'a, DiscoveryResponse> {
            Box::pin(async move {
                self.discoveries.fetch_add(1, Ordering::SeqCst);
                self.events.lock().unwrap().push("discover");
                self.discovery.lock().unwrap().take().unwrap_or_else(|| {
                    Ok(DiscoveryResponse::fixture(
                        "gpt-pro",
                        "gpt-5.6-sol",
                        &["gpt-5.6-sol"],
                    ))
                })
            })
        }

        fn revoke<'a>(
            &'a self,
            _credential: &'a SecretCredential,
            _base_url: &'a str,
        ) -> TransportFuture<'a, ()> {
            Box::pin(async move {
                self.revokes.fetch_add(1, Ordering::SeqCst);
                if self.revoke_fails {
                    return Err("token_revoke_failed".to_string());
                }
                Ok(())
            })
        }
    }

    #[derive(Default)]
    struct FakeCredentialStore(Mutex<Option<String>>);

    impl CredentialStore for FakeCredentialStore {
        fn read(&self) -> Result<Option<SecretCredential>, String> {
            Ok(self.0.lock().unwrap().clone().map(SecretCredential::new))
        }

        fn write(&self, credential: &SecretCredential) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(credential.expose().to_string());
            Ok(())
        }

        fn delete(&self) -> Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeMetadataStore(Mutex<Option<SafeAccountMetadata>>);

    impl super::AccountMetadataStore for FakeMetadataStore {
        fn load(&self) -> Result<Option<SafeAccountMetadata>, String> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn save(&self, metadata: &SafeAccountMetadata) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(metadata.clone());
            Ok(())
        }

        fn clear(&self) -> Result<(), String> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    struct FailWriteCredentialStore;

    struct BlockingCredentialStore {
        released: Arc<(Mutex<bool>, Condvar)>,
    }

    impl CredentialStore for BlockingCredentialStore {
        fn read(&self) -> Result<Option<SecretCredential>, String> {
            let (lock, wake) = &*self.released;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
            Ok(None)
        }

        fn write(&self, _credential: &SecretCredential) -> Result<(), String> {
            Ok(())
        }

        fn delete(&self) -> Result<(), String> {
            Ok(())
        }
    }

    impl CredentialStore for FailWriteCredentialStore {
        fn read(&self) -> Result<Option<SecretCredential>, String> {
            Ok(None)
        }

        fn write(&self, _credential: &SecretCredential) -> Result<(), String> {
            Err("credential_store_write_failed".to_string())
        }

        fn delete(&self) -> Result<(), String> {
            Ok(())
        }
    }

    struct FailDeleteCredentialStore;

    impl CredentialStore for FailDeleteCredentialStore {
        fn read(&self) -> Result<Option<SecretCredential>, String> {
            Ok(Some(SecretCredential::new("secret".to_string())))
        }

        fn write(&self, _credential: &SecretCredential) -> Result<(), String> {
            Ok(())
        }

        fn delete(&self) -> Result<(), String> {
            Err("credential_store_delete_failed".to_string())
        }
    }

    struct TracingCredentialStore {
        value: Mutex<Option<String>>,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl CredentialStore for TracingCredentialStore {
        fn read(&self) -> Result<Option<SecretCredential>, String> {
            Ok(self
                .value
                .lock()
                .unwrap()
                .clone()
                .map(SecretCredential::new))
        }

        fn write(&self, credential: &SecretCredential) -> Result<(), String> {
            self.events.lock().unwrap().push("credential_write");
            *self.value.lock().unwrap() = Some(credential.expose().to_string());
            Ok(())
        }

        fn delete(&self) -> Result<(), String> {
            *self.value.lock().unwrap() = None;
            Ok(())
        }
    }

    fn fake_transport(
        polls: impl IntoIterator<Item = Result<PollResponse, String>>,
        revoke_fails: bool,
    ) -> Arc<FakeTransport> {
        Arc::new(FakeTransport {
            polls: Mutex::new(polls.into_iter().collect()),
            discovery: Mutex::new(None),
            discoveries: AtomicUsize::new(0),
            events: Arc::new(Mutex::new(Vec::new())),
            revokes: AtomicUsize::new(0),
            revoke_fails,
        })
    }

    fn service_with(
        transport: Arc<FakeTransport>,
        credentials: Arc<dyn CredentialStore>,
    ) -> Arc<AccountService> {
        Arc::new(AccountService::from_parts(
            transport,
            credentials,
            Arc::new(FakeMetadataStore::default()),
        ))
    }

    fn stored_service_with(
        transport: Arc<FakeTransport>,
    ) -> (
        Arc<AccountService>,
        Arc<FakeCredentialStore>,
        Arc<FakeMetadataStore>,
    ) {
        let credentials = Arc::new(FakeCredentialStore(Mutex::new(Some(
            "fixture-secret-marker-never-public".to_string(),
        ))));
        let metadata = Arc::new(FakeMetadataStore(Mutex::new(Some(SafeAccountMetadata {
            email: "ender@example.com".to_string(),
            group: "gpt-pro".to_string(),
            default_model: "gpt-5.6-sol".to_string(),
            allowed_models: vec!["gpt-5.6-sol".to_string()],
            key_name: "Beefex".to_string(),
            base_url: "https://beefapi.com/v1".to_string(),
        }))));
        (
            Arc::new(AccountService::from_parts(
                transport,
                credentials.clone(),
                metadata.clone(),
            )),
            credentials,
            metadata,
        )
    }

    fn auth_fixture_expected(case_id: &str, field: &str) -> serde_json::Value {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/beefapi-desktop-auth-v1.json"
        ))
        .unwrap();
        fixture["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == case_id)
            .unwrap()["expected"][field]
            .clone()
    }

    #[tokio::test(start_paused = true)]
    async fn approved_auth_keeps_secret_out_of_public_state() {
        const SECRET: &str = "fixture-secret-marker-never-public";
        let credentials = Arc::new(FakeCredentialStore::default());
        let service = service_with(
            fake_transport(
                [Ok(PollResponse::Approved(ManagedCredential::fixture(
                    SECRET,
                    "https://beefapi.com/v1",
                    "ender@example.com",
                    "Beefex - test",
                    "gpt-pro",
                )))],
                false,
            ),
            credentials.clone(),
        );

        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .expect("device authorization starts");
        let public_authorization = serde_json::to_string(&service.state()).unwrap();
        assert!(public_authorization.contains("BEEF-CODE"));
        assert!(!public_authorization.contains("device-private"));
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::time::advance(Duration::from_secs(3)).await;
        let state = poller.await.unwrap().expect("authorization completes");

        assert_eq!(state.phase, AccountPhase::SignedIn);
        assert_eq!(state.email.as_deref(), Some("ender@example.com"));
        assert_eq!(state.group.as_deref(), Some("gpt-pro"));
        assert_eq!(state.default_model.as_deref(), Some("gpt-5.6-sol"));
        assert_eq!(credentials.0.lock().unwrap().as_deref(), Some(SECRET));
        assert!(!serde_json::to_string(&state).unwrap().contains(SECRET));
        assert_eq!(
            auth_fixture_expected("authorized_pro_default_model_available", "reason_code"),
            "authorized"
        );
        let provider = service
            .resolve_managed_provider(None, |_| {})
            .await
            .expect("authorized account hydrates the managed provider")
            .into_inner();
        assert_eq!(
            auth_fixture_expected(
                "authorized_pro_default_model_available",
                "inference_allowed"
            ),
            true
        );
        assert_eq!(
            auth_fixture_expected("authorized_pro_default_model_available", "effective_group"),
            state.group.unwrap()
        );
        assert_eq!(
            auth_fixture_expected("authorized_pro_default_model_available", "effective_model"),
            provider.enabled_models[0]
        );
        assert_eq!(provider.id, crate::beefapi::provider::MANAGED_PROVIDER_ID);
        assert_eq!(provider.base_url, "https://beefapi.com/v1");
    }

    #[tokio::test]
    async fn start_network_failure_enters_offline_state() {
        let service = AccountService::from_parts(
            Arc::new(OfflineStartTransport),
            Arc::new(FakeCredentialStore::default()),
            Arc::new(FakeMetadataStore::default()),
        );

        let error = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .err()
            .expect("start fails");
        assert_eq!(error, "network_unavailable");
        assert_eq!(service.state().phase, AccountPhase::Offline);
        assert_eq!(
            service.state().reason.as_deref(),
            Some("network_unavailable")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn pending_then_denied_is_terminal() {
        let service = service_with(
            fake_transport([Ok(PollResponse::Pending), Ok(PollResponse::Denied)], false),
            Arc::new(FakeCredentialStore::default()),
        );
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3)).await;
        tokio::task::yield_now().await;
        assert_eq!(service.state().phase, AccountPhase::Polling);
        tokio::time::advance(Duration::from_secs(3)).await;

        let state = poller.await.unwrap().unwrap();
        assert_eq!(state.phase, AccountPhase::Denied);
        assert_eq!(state.reason.as_deref(), Some("authorization_denied"));
    }

    #[tokio::test(start_paused = true)]
    async fn expired_entitlement_and_default_model_fail_closed() {
        for (response, phase, reason) in [
            (
                PollResponse::Expired,
                AccountPhase::Expired,
                "authorization_expired",
            ),
            (
                PollResponse::EntitlementMissing,
                AccountPhase::EntitlementMissing,
                "entitlement_required",
            ),
            (
                PollResponse::DefaultModelUnavailable,
                AccountPhase::SignedOut,
                "default_model_unavailable",
            ),
        ] {
            let service = service_with(
                fake_transport([Ok(response)], false),
                Arc::new(FakeCredentialStore::default()),
            );
            let authorization = service
                .begin_authorization("0.1.0", "test-mac")
                .await
                .unwrap();
            let poller = {
                let service = service.clone();
                tokio::spawn(
                    async move { service.complete_authorization(authorization, |_| {}).await },
                )
            };
            tokio::task::yield_now().await;
            tokio::time::advance(Duration::from_secs(3)).await;
            let state = poller.await.unwrap().unwrap();

            assert_eq!(state.phase, phase);
            assert_eq!(state.reason.as_deref(), Some(reason));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn slow_down_adds_five_seconds_to_subsequent_polling() {
        let service = service_with(
            fake_transport(
                [
                    Ok(PollResponse::SlowDown),
                    Ok(PollResponse::Approved(ManagedCredential::fixture(
                        "secret",
                        "https://beefapi.com/v1",
                        "ender@example.com",
                        "Beefex",
                        "gpt-pro",
                    ))),
                ],
                false,
            ),
            Arc::new(FakeCredentialStore::default()),
        );
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::time::advance(Duration::from_secs(3)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(7)).await;
        tokio::task::yield_now().await;
        assert!(!poller.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;

        assert_eq!(poller.await.unwrap().unwrap().phase, AccountPhase::SignedIn);
    }

    #[tokio::test(start_paused = true)]
    async fn offline_poll_recovers_without_restarting_authorization() {
        let service = service_with(
            fake_transport(
                [
                    Err("network_unavailable".to_string()),
                    Ok(PollResponse::Approved(ManagedCredential::fixture(
                        "secret",
                        "https://beefapi.com/v1",
                        "ender@example.com",
                        "Beefex",
                        "gpt-pro",
                    ))),
                ],
                false,
            ),
            Arc::new(FakeCredentialStore::default()),
        );
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3)).await;
        tokio::task::yield_now().await;
        assert_eq!(service.state().phase, AccountPhase::Offline);
        tokio::time::advance(Duration::from_secs(6)).await;

        assert_eq!(poller.await.unwrap().unwrap().phase, AccountPhase::SignedIn);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_invalidates_the_active_poll_generation() {
        let credentials = Arc::new(FakeCredentialStore::default());
        let service = service_with(
            fake_transport(
                [Ok(PollResponse::Approved(ManagedCredential::fixture(
                    "late-secret",
                    "https://beefapi.com/v1",
                    "ender@example.com",
                    "Beefex",
                    "gpt-pro",
                )))],
                false,
            ),
            credentials.clone(),
        );
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::task::yield_now().await;
        service.cancel_authorization();
        tokio::time::advance(Duration::from_secs(3)).await;

        assert_eq!(
            poller.await.unwrap().unwrap().phase,
            AccountPhase::SignedOut
        );
        assert_eq!(credentials.0.lock().unwrap().as_deref(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn untrusted_base_url_revokes_without_persisting_secret() {
        let transport = fake_transport(
            [Ok(PollResponse::Approved(ManagedCredential::fixture(
                "secret",
                "https://attacker.example/v1",
                "ender@example.com",
                "Beefex",
                "gpt-pro",
            )))],
            false,
        );
        let credentials = Arc::new(FakeCredentialStore::default());
        let service = service_with(transport.clone(), credentials.clone());
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::time::advance(Duration::from_secs(3)).await;
        let state = poller.await.unwrap().unwrap();

        assert_eq!(state.phase, AccountPhase::SignedOut);
        assert_eq!(state.reason.as_deref(), Some("untrusted_base_url"));
        assert_eq!(
            auth_fixture_expected("untrusted_base_url_rejected_by_client", "reason_code"),
            state.reason.unwrap()
        );
        assert_eq!(transport.revokes.load(Ordering::SeqCst), 1);
        assert!(credentials.0.lock().unwrap().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn credential_write_failure_revokes_and_never_signs_in() {
        let transport = fake_transport(
            [Ok(PollResponse::Approved(ManagedCredential::fixture(
                "secret",
                "https://beefapi.com/v1",
                "ender@example.com",
                "Beefex",
                "gpt-pro",
            )))],
            false,
        );
        let service = service_with(transport.clone(), Arc::new(FailWriteCredentialStore));
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::time::advance(Duration::from_secs(3)).await;
        let state = poller.await.unwrap().unwrap();

        assert_eq!(state.phase, AccountPhase::CredentialStoreFailed);
        assert_eq!(
            auth_fixture_expected("keychain_final_write_failure_revokes_token", "reason_code"),
            "credential_store_failed"
        );
        assert_eq!(transport.revokes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn discovery_is_validated_before_the_final_credential_write() {
        let transport = fake_transport(
            [Ok(PollResponse::Approved(ManagedCredential::fixture(
                "secret",
                "https://beefapi.com/v1",
                "ender@example.com",
                "Beefex",
                "gpt-pro",
            )))],
            false,
        );
        let credentials = Arc::new(TracingCredentialStore {
            value: Mutex::new(None),
            events: transport.events.clone(),
        });
        let service = service_with(transport.clone(), credentials);
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3)).await;

        assert_eq!(poller.await.unwrap().unwrap().phase, AccountPhase::SignedIn);
        assert_eq!(
            transport.events.lock().unwrap().as_slice(),
            ["discover", "credential_write"]
        );
    }

    #[tokio::test]
    async fn startup_reconciles_and_offline_preserves_the_local_credential() {
        let transport = fake_transport([], false);
        let (service, credentials, metadata) = stored_service_with(transport.clone());

        assert_eq!(
            service.reconcile(|_| {}).await.phase,
            AccountPhase::SignedIn
        );
        assert_eq!(transport.discoveries.load(Ordering::SeqCst), 1);

        *transport.discovery.lock().unwrap() = Some(Err("network_unavailable".to_string()));
        assert_eq!(service.reconcile(|_| {}).await.phase, AccountPhase::Offline);
        assert!(credentials.0.lock().unwrap().is_some());
        assert!(metadata.0.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn startup_credential_read_timeout_fails_closed_instead_of_staying_initializing() {
        let released = Arc::new((Mutex::new(false), Condvar::new()));
        let service = Arc::new(service_with(
            fake_transport([], false),
            Arc::new(BlockingCredentialStore {
                released: released.clone(),
            }),
        ));
        let reconcile = {
            let service = service.clone();
            tokio::spawn(async move { service.reconcile(|_| {}).await })
        };

        let state = reconcile.await.unwrap();

        assert_eq!(state.phase, AccountPhase::CredentialStoreFailed);
        assert_eq!(
            state.reason.as_deref(),
            Some("credential_store_read_timeout")
        );
        let (lock, wake) = &*released;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }

    #[tokio::test]
    async fn runtime_credential_rejection_clears_local_state_and_never_falls_back() {
        let transport = fake_transport([], false);
        let (service, credentials, metadata) = stored_service_with(transport.clone());
        assert_eq!(
            service.reconcile(|_| {}).await.phase,
            AccountPhase::SignedIn
        );

        let state = service.handle_inference_credential_rejected(|_| {}).await;

        assert_eq!(state.phase, AccountPhase::SignedOut);
        assert_eq!(state.reason.as_deref(), Some("reauthorization_required"));
        assert_eq!(
            auth_fixture_expected(
                "runtime_credential_rejected_requires_reauthorization",
                "reason_code"
            ),
            state.reason.clone().unwrap()
        );
        assert!(credentials.0.lock().unwrap().is_none());
        assert!(metadata.0.lock().unwrap().is_none());
        assert_eq!(transport.revokes.load(Ordering::SeqCst), 1);
        assert_eq!(
            service
                .resolve_managed_provider(None, |_| {})
                .await
                .unwrap_err(),
            "reauthorization_required"
        );
    }

    #[tokio::test]
    async fn logout_deletes_the_credential_and_disables_managed_resolution() {
        let transport = fake_transport([], false);
        let (service, credentials, metadata) = stored_service_with(transport.clone());
        assert_eq!(
            service.reconcile(|_| {}).await.phase,
            AccountPhase::SignedIn
        );

        let state = service.logout(|_| {}).await;

        assert_eq!(state.phase, AccountPhase::SignedOut);
        assert_eq!(state.reason.as_deref(), Some("logged_out"));
        assert!(credentials.0.lock().unwrap().is_none());
        assert!(metadata.0.lock().unwrap().is_none());
        assert_eq!(transport.revokes.load(Ordering::SeqCst), 1);
        assert_eq!(
            service
                .resolve_managed_provider(None, |_| {})
                .await
                .unwrap_err(),
            "reauthorization_required"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn failed_cleanup_surfaces_cleanup_required() {
        let transport = fake_transport(
            [Ok(PollResponse::Approved(ManagedCredential::fixture(
                "secret",
                "https://beefapi.com/v1",
                "ender@example.com",
                "Beefex",
                "wrong-group",
            )))],
            true,
        );
        let service = service_with(transport.clone(), Arc::new(FakeCredentialStore::default()));
        let authorization = service
            .begin_authorization("0.1.0", "test-mac")
            .await
            .unwrap();
        let poller = {
            let service = service.clone();
            tokio::spawn(async move { service.complete_authorization(authorization, |_| {}).await })
        };
        tokio::time::advance(Duration::from_secs(3)).await;
        let state = poller.await.unwrap().unwrap();

        assert_eq!(state.phase, AccountPhase::CleanupRequired);
        assert_eq!(state.reason.as_deref(), Some("cleanup_required"));
        assert_eq!(transport.revokes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn local_delete_failure_never_reports_signed_out_cleanup() {
        let transport = fake_transport([], false);
        let metadata = Arc::new(FakeMetadataStore(Mutex::new(Some(SafeAccountMetadata {
            email: "ender@example.com".to_string(),
            group: "gpt-pro".to_string(),
            default_model: "gpt-5.6-sol".to_string(),
            allowed_models: vec!["gpt-5.6-sol".to_string()],
            key_name: "Beefex".to_string(),
            base_url: "https://beefapi.com/v1".to_string(),
        }))));
        let service = AccountService::from_parts(
            transport.clone(),
            Arc::new(FailDeleteCredentialStore),
            metadata,
        );

        let state = service.handle_inference_credential_rejected(|_| {}).await;

        assert_eq!(state.phase, AccountPhase::CleanupRequired);
        assert_eq!(state.reason.as_deref(), Some("cleanup_required"));
        assert_eq!(transport.revokes.load(Ordering::SeqCst), 1);
    }
}

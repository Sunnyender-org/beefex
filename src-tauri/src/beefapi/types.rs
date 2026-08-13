use serde::{Deserialize, Serialize};

use super::credential_store::SecretCredential;

pub(crate) const CLIENT_ID: &str = "beefex-desktop-v1";
pub(crate) const REQUIRED_GROUP: &str = "gpt-pro";
pub(crate) const REQUIRED_DEFAULT_MODEL: &str = "gpt-5.6-sol";
pub(crate) const PRODUCTION_BASE_URL: &str = "https://beefapi.com/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountPhase {
    SignedOut,
    Authorizing,
    Polling,
    SignedIn,
    Denied,
    Expired,
    Offline,
    EntitlementMissing,
    CredentialStoreFailed,
    CleanupRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountState {
    pub phase: AccountPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AccountState {
    pub(crate) fn signed_out(reason: Option<&str>) -> Self {
        Self {
            phase: AccountPhase::SignedOut,
            email: None,
            group: None,
            default_model: None,
            allowed_models: Vec::new(),
            key_name: None,
            user_code: None,
            verification_uri: None,
            verification_uri_complete: None,
            expires_at: None,
            reason: reason.map(str::to_string),
        }
    }

    pub(crate) fn failure(phase: AccountPhase, reason: &str) -> Self {
        Self {
            phase,
            reason: Some(reason.to_string()),
            ..Self::signed_out(None)
        }
    }

    pub(crate) fn signed_in(metadata: &SafeAccountMetadata) -> Self {
        Self::from_metadata(metadata, AccountPhase::SignedIn, None)
    }

    pub(crate) fn from_metadata(
        metadata: &SafeAccountMetadata,
        phase: AccountPhase,
        reason: Option<&str>,
    ) -> Self {
        Self {
            phase,
            email: Some(metadata.email.clone()),
            group: Some(metadata.group.clone()),
            default_model: Some(metadata.default_model.clone()),
            allowed_models: if metadata.allowed_models.is_empty() {
                vec![metadata.default_model.clone()]
            } else {
                metadata.allowed_models.clone()
            },
            key_name: Some(metadata.key_name.clone()),
            user_code: None,
            verification_uri: None,
            verification_uri_complete: None,
            expires_at: None,
            reason: reason.map(str::to_string),
        }
    }
}

impl Default for AccountState {
    fn default() -> Self {
        Self::signed_out(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(crate) struct SafeAccountMetadata {
    pub email: String,
    pub group: String,
    pub default_model: String,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    pub key_name: String,
    pub base_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AuthStartResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

pub(crate) struct ManagedCredential {
    pub credential: SecretCredential,
    pub base_url: String,
    pub user_email: String,
    pub key_name: String,
    pub group: String,
}

impl std::fmt::Debug for ManagedCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ManagedCredential")
            .field("credential", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("user_email", &self.user_email)
            .field("key_name", &self.key_name)
            .field("group", &self.group)
            .finish()
    }
}

#[cfg(test)]
impl ManagedCredential {
    pub(crate) fn fixture(
        secret: &str,
        base_url: &str,
        user_email: &str,
        key_name: &str,
        group: &str,
    ) -> Self {
        Self {
            credential: SecretCredential::new(secret.to_string()),
            base_url: base_url.to_string(),
            user_email: user_email.to_string(),
            key_name: key_name.to_string(),
            group: group.to_string(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum PollResponse {
    Pending,
    SlowDown,
    Approved(ManagedCredential),
    Denied,
    Expired,
    EntitlementMissing,
    DefaultModelUnavailable,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscoveryResponse {
    pub success: bool,
    pub default_group: String,
    pub default_model: String,
    #[serde(default)]
    pub groups: Vec<DiscoveryGroup>,
    #[serde(default)]
    pub models: Vec<DiscoveryModel>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscoveryGroup {
    pub id: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct DiscoveryModel {
    pub id: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub available: bool,
}

#[cfg(test)]
impl DiscoveryResponse {
    pub(crate) fn fixture(group: &str, default_model: &str, models: &[&str]) -> Self {
        Self {
            success: true,
            default_group: group.to_string(),
            default_model: default_model.to_string(),
            groups: vec![DiscoveryGroup {
                id: group.to_string(),
                enabled: true,
            }],
            models: models
                .iter()
                .map(|model| DiscoveryModel {
                    id: (*model).to_string(),
                    groups: vec![group.to_string()],
                    available: true,
                })
                .collect(),
        }
    }
}

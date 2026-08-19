use std::collections::{BTreeMap, HashMap, HashSet};

use url::Url;

use crate::settings::ModelProvider;

use super::types::{
    AccountPhase, AccountState, DiscoveryResponse, ManagedCredential, SafeAccountMetadata,
    PRODUCTION_BASE_URL, REQUIRED_DEFAULT_MODEL, REQUIRED_GROUP,
};

pub(crate) const MANAGED_PROVIDER_ID: &str = "beefapi-managed";

pub(crate) fn is_managed_provider_id(provider_id: &str) -> bool {
    provider_id == MANAGED_PROVIDER_ID
}

pub(crate) fn managed_model_selection(state: &AccountState) -> Option<(String, String)> {
    let usable_phase = matches!(state.phase, AccountPhase::SignedIn | AccountPhase::Offline);
    let trusted_metadata = state.group.as_deref() == Some(REQUIRED_GROUP)
        && state.default_model.as_deref() == Some(REQUIRED_DEFAULT_MODEL);
    (usable_phase && trusted_metadata).then(|| {
        (
            MANAGED_PROVIDER_ID.to_string(),
            REQUIRED_DEFAULT_MODEL.to_string(),
        )
    })
}

pub(crate) fn validate_managed_credential(
    credential: &ManagedCredential,
) -> Result<(), &'static str> {
    if credential.group != REQUIRED_GROUP {
        return Err("unsupported_token_group");
    }
    validate_managed_base_url(&credential.base_url)
}

pub(crate) fn validate_managed_base_url(candidate: &str) -> Result<(), &'static str> {
    let parsed = Url::parse(candidate).map_err(|_| "untrusted_base_url")?;
    if parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/v1"
    {
        return Err("untrusted_base_url");
    }
    if candidate == PRODUCTION_BASE_URL {
        return Ok(());
    }
    #[cfg(debug_assertions)]
    {
        let host = parsed.host_str().unwrap_or_default();
        if parsed.scheme() == "http" && (host == "127.0.0.1" || host == "localhost") {
            return Ok(());
        }
    }
    Err("untrusted_base_url")
}

pub(crate) fn validate_discovery(
    credential: &ManagedCredential,
    discovery: &DiscoveryResponse,
) -> Result<SafeAccountMetadata, &'static str> {
    if !discovery.success || discovery.default_group != credential.group {
        return Err("unsupported_token_group");
    }
    if discovery.default_group != REQUIRED_GROUP {
        return Err("unsupported_token_group");
    }
    if discovery.default_model != REQUIRED_DEFAULT_MODEL {
        return Err("default_model_unavailable");
    }
    let enabled_groups: HashSet<&str> = discovery
        .groups
        .iter()
        .filter(|group| group.enabled && !group.id.trim().is_empty())
        .map(|group| group.id.as_str())
        .collect();
    if !enabled_groups.contains(REQUIRED_GROUP) {
        return Err("unsupported_token_group");
    }
    if !discovery.models.iter().any(|model| {
        model.id == discovery.default_model
            && model.available
            && model.groups.iter().any(|group| group == REQUIRED_GROUP)
    }) {
        return Err("default_model_unavailable");
    }
    let mut allowed_models = Vec::new();
    let mut model_groups = BTreeMap::new();
    for model in &discovery.models {
        if !model.available {
            continue;
        }
        let Some(group) = select_routing_group(&model.groups, &enabled_groups) else {
            continue;
        };
        if !allowed_models.iter().any(|id| id == &model.id) {
            allowed_models.push(model.id.clone());
        }
        model_groups.insert(model.id.clone(), group);
    }
    Ok(SafeAccountMetadata {
        email: credential.user_email.clone(),
        group: credential.group.clone(),
        default_model: discovery.default_model.clone(),
        allowed_models,
        model_groups,
        key_name: credential.key_name.clone(),
        base_url: credential.base_url.clone(),
    })
}

fn select_routing_group(model_groups: &[String], enabled_groups: &HashSet<&str>) -> Option<String> {
    if model_groups
        .iter()
        .any(|group| group == REQUIRED_GROUP && enabled_groups.contains(REQUIRED_GROUP))
    {
        return Some(REQUIRED_GROUP.to_string());
    }
    model_groups
        .iter()
        .find(|group| enabled_groups.contains(group.as_str()))
        .cloned()
}

pub(crate) struct EphemeralModelProvider {
    inner: ModelProvider,
    routing_group: String,
}

impl EphemeralModelProvider {
    pub(crate) fn model(&self) -> Option<&str> {
        self.inner.enabled_models.first().map(String::as_str)
    }

    pub(crate) fn routing_group(&self) -> &str {
        &self.routing_group
    }

    pub(crate) fn into_inner(self) -> ModelProvider {
        self.inner
    }
}

impl std::fmt::Debug for EphemeralModelProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EphemeralModelProvider")
            .field("id", &self.inner.id)
            .field("base_url", &self.inner.base_url)
            .field("models", &self.inner.enabled_models)
            .field("routing_group", &self.routing_group)
            .field("credential", &"<redacted>")
            .finish()
    }
}

pub(crate) fn hydrate_managed_provider(
    metadata: &SafeAccountMetadata,
    credential: &super::credential_store::SecretCredential,
    requested_model: Option<&str>,
) -> Result<EphemeralModelProvider, &'static str> {
    let allowed_models = if metadata.allowed_models.is_empty() {
        vec![metadata.default_model.clone()]
    } else {
        metadata.allowed_models.clone()
    };
    let selected = requested_model.unwrap_or(&metadata.default_model);
    if !allowed_models.iter().any(|model| model == selected) {
        return Err("model_not_allowed");
    }
    Ok(EphemeralModelProvider {
        routing_group: metadata.routing_group_for(selected).to_string(),
        inner: ModelProvider {
            id: MANAGED_PROVIDER_ID.to_string(),
            name: "BeefAPI".to_string(),
            api_keys: vec![credential.expose().to_string()],
            api_key_legacy: None,
            base_url: metadata.base_url.clone(),
            available_models: allowed_models,
            enabled_models: vec![selected.to_string()],
            enabled: true,
            api_format: "openai_responses".to_string(),
            model_overrides: HashMap::new(),
            compress_request_body: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::beefapi::types::DiscoveryResponse;

    #[test]
    fn production_base_url_is_exact() {
        assert_eq!(validate_managed_base_url(PRODUCTION_BASE_URL), Ok(()));
        assert_eq!(
            validate_managed_base_url("https://beefapi.com.evil.example/v1"),
            Err("untrusted_base_url")
        );
        assert_eq!(
            validate_managed_base_url("https://user@beefapi.com/v1"),
            Err("untrusted_base_url")
        );
    }

    #[test]
    fn discovery_requires_server_default_to_be_available() {
        let credential = ManagedCredential::fixture(
            "secret",
            PRODUCTION_BASE_URL,
            "ender@example.com",
            "Beefex",
            REQUIRED_GROUP,
        );
        let discovery =
            DiscoveryResponse::fixture(REQUIRED_GROUP, REQUIRED_DEFAULT_MODEL, &["gpt-5.5"]);
        assert_eq!(
            validate_discovery(&credential, &discovery),
            Err("default_model_unavailable")
        );
    }

    #[test]
    fn discovery_projects_all_available_models_from_enabled_groups() {
        let credential = ManagedCredential::fixture(
            "secret",
            PRODUCTION_BASE_URL,
            "ender@example.com",
            "Beefex",
            REQUIRED_GROUP,
        );
        let mut discovery = DiscoveryResponse::fixture(
            REQUIRED_GROUP,
            REQUIRED_DEFAULT_MODEL,
            &[REQUIRED_DEFAULT_MODEL, "gpt-5.5", "disabled-model"],
        );
        discovery.groups.push(super::super::types::DiscoveryGroup {
            id: "claude max".to_string(),
            enabled: true,
        });
        discovery.models[2].available = false;
        discovery.models.push(super::super::types::DiscoveryModel {
            id: "claude-fable-5".to_string(),
            groups: vec!["claude max".to_string()],
            available: true,
        });
        discovery.models.push(super::super::types::DiscoveryModel {
            id: "studio-secret".to_string(),
            groups: vec!["studio".to_string()],
            available: true,
        });
        let metadata = validate_discovery(&credential, &discovery).unwrap();
        assert_eq!(
            metadata.allowed_models,
            vec![
                REQUIRED_DEFAULT_MODEL.to_string(),
                "gpt-5.5".to_string(),
                "claude-fable-5".to_string()
            ]
        );
        assert_eq!(
            metadata
                .model_groups
                .get("claude-fable-5")
                .map(String::as_str),
            Some("claude max")
        );
        assert_eq!(
            metadata
                .model_groups
                .get(REQUIRED_DEFAULT_MODEL)
                .map(String::as_str),
            Some(REQUIRED_GROUP)
        );
        assert!(!metadata
            .allowed_models
            .iter()
            .any(|model| model == "studio-secret"));
    }

    #[test]
    fn routing_prefers_required_group_when_a_model_is_in_multiple_groups() {
        let credential = ManagedCredential::fixture(
            "secret",
            PRODUCTION_BASE_URL,
            "ender@example.com",
            "Beefex",
            REQUIRED_GROUP,
        );
        let mut discovery = DiscoveryResponse::fixture(
            REQUIRED_GROUP,
            REQUIRED_DEFAULT_MODEL,
            &[REQUIRED_DEFAULT_MODEL],
        );
        discovery.groups.push(super::super::types::DiscoveryGroup {
            id: "gpt-plus".to_string(),
            enabled: true,
        });
        discovery.models.push(super::super::types::DiscoveryModel {
            id: "gpt-5.5".to_string(),
            groups: vec!["gpt-plus".to_string(), REQUIRED_GROUP.to_string()],
            available: true,
        });
        let metadata = validate_discovery(&credential, &discovery).unwrap();
        assert_eq!(metadata.routing_group_for("gpt-5.5"), REQUIRED_GROUP);
    }

    #[test]
    fn hydration_rejects_models_outside_the_server_allowlist() {
        let metadata = SafeAccountMetadata {
            email: "ender@example.com".to_string(),
            group: REQUIRED_GROUP.to_string(),
            default_model: REQUIRED_DEFAULT_MODEL.to_string(),
            allowed_models: vec![REQUIRED_DEFAULT_MODEL.to_string()],
            model_groups: BTreeMap::new(),
            key_name: "Beefex".to_string(),
            base_url: PRODUCTION_BASE_URL.to_string(),
        };
        let result = hydrate_managed_provider(
            &metadata,
            &super::super::credential_store::SecretCredential::new("secret".to_string()),
            Some("not-allowed"),
        );
        assert!(matches!(result, Err("model_not_allowed")));
    }

    #[test]
    fn ephemeral_provider_debug_never_contains_the_credential() {
        const SECRET: &str = "fixture-secret-marker-never-public";
        let metadata = SafeAccountMetadata {
            email: "ender@example.com".to_string(),
            group: REQUIRED_GROUP.to_string(),
            default_model: REQUIRED_DEFAULT_MODEL.to_string(),
            allowed_models: vec![REQUIRED_DEFAULT_MODEL.to_string()],
            model_groups: BTreeMap::new(),
            key_name: "Beefex".to_string(),
            base_url: PRODUCTION_BASE_URL.to_string(),
        };
        let provider = hydrate_managed_provider(
            &metadata,
            &super::super::credential_store::SecretCredential::new(SECRET.to_string()),
            None,
        )
        .unwrap();

        let debug = format!("{provider:?}");
        assert!(!debug.contains(SECRET));
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn hydration_uses_the_selected_model_routing_group() {
        let mut model_groups = BTreeMap::new();
        model_groups.insert("claude-fable-5".to_string(), "claude max".to_string());
        let metadata = SafeAccountMetadata {
            email: "ender@example.com".to_string(),
            group: REQUIRED_GROUP.to_string(),
            default_model: REQUIRED_DEFAULT_MODEL.to_string(),
            allowed_models: vec![
                REQUIRED_DEFAULT_MODEL.to_string(),
                "claude-fable-5".to_string(),
            ],
            model_groups,
            key_name: "Beefex".to_string(),
            base_url: PRODUCTION_BASE_URL.to_string(),
        };
        let provider = hydrate_managed_provider(
            &metadata,
            &super::super::credential_store::SecretCredential::new("secret".to_string()),
            Some("claude-fable-5"),
        )
        .unwrap();
        assert_eq!(provider.model(), Some("claude-fable-5"));
        assert_eq!(provider.routing_group(), "claude max");
    }

    #[test]
    fn managed_model_is_default_only_after_verified_account_metadata_exists() {
        assert_eq!(
            managed_model_selection(&AccountState::signed_out(None)),
            None
        );
        let state = AccountState::signed_in(&SafeAccountMetadata {
            email: "ender@example.com".to_string(),
            group: REQUIRED_GROUP.to_string(),
            default_model: REQUIRED_DEFAULT_MODEL.to_string(),
            allowed_models: vec![REQUIRED_DEFAULT_MODEL.to_string()],
            model_groups: BTreeMap::new(),
            key_name: "Beefex".to_string(),
            base_url: PRODUCTION_BASE_URL.to_string(),
        });
        assert_eq!(
            managed_model_selection(&state),
            Some((
                MANAGED_PROVIDER_ID.to_string(),
                REQUIRED_DEFAULT_MODEL.to_string()
            ))
        );
    }
}

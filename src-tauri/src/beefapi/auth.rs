use std::time::Duration;

use reqwest::{redirect::Policy, Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    account::{AccountTransport, TransportFuture},
    credential_store::{FileCredentialStore, SecretCredential},
    types::{
        AuthStartResponse, DiscoveryResponse, ManagedClientCredential, ManagedClientCredentials,
        ManagedCredential, PollResponse, CLIENT_ID, REQUIRED_GROUP,
    },
};

const PRODUCTION_ORIGIN: &str = "https://beefapi.com";

pub(crate) struct BeefApiClient {
    http: Client,
    origin: Url,
    base_url: String,
}

impl BeefApiClient {
    pub(crate) fn new() -> Result<Self, String> {
        let origin = configured_origin()?;
        Ok(Self {
            http: Client::builder()
                .redirect(Policy::none())
                .timeout(Duration::from_secs(30))
                .user_agent("Beefex/0.1")
                .build()
                .map_err(|_| "beefapi_client_unavailable".to_string())?,
            base_url: format!("{}/v1", origin.as_str().trim_end_matches('/')),
            origin,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, String> {
        self.origin
            .join(path)
            .map_err(|_| "beefapi_endpoint_invalid".to_string())
    }

    fn validate_authorization_url(&self, raw: &str, complete: bool) -> Result<(), String> {
        let candidate =
            Url::parse(raw).map_err(|_| "invalid_authorization_response".to_string())?;
        if candidate.scheme() != self.origin.scheme()
            || candidate.host_str() != self.origin.host_str()
            || candidate.port_or_known_default() != self.origin.port_or_known_default()
            || candidate.username() != ""
            || candidate.password().is_some()
            || candidate.fragment().is_some()
            || candidate.path() != "/desktop-auth"
        {
            return Err("invalid_authorization_response".to_string());
        }
        if complete {
            if candidate
                .query_pairs()
                .any(|(key, value)| key == "code" && !value.trim().is_empty())
            {
                return Ok(());
            }
            return Err("invalid_authorization_response".to_string());
        }
        if candidate.query().is_some() {
            return Err("invalid_authorization_response".to_string());
        }
        Ok(())
    }
}

impl AccountTransport for BeefApiClient {
    fn trusted_base_url(&self) -> &str {
        &self.base_url
    }

    fn start<'a>(
        &'a self,
        client_version: &'a str,
        hostname: &'a str,
    ) -> TransportFuture<'a, AuthStartResponse> {
        Box::pin(async move {
            let response = self
                .http
                .post(self.endpoint("api/oauth/device/code")?)
                .json(&StartRequest {
                    client_id: CLIENT_ID,
                    client_version,
                    hostname,
                    scope: "inference",
                    preferred_group: REQUIRED_GROUP,
                })
                .send()
                .await
                .map_err(|_| "network_unavailable".to_string())?;
            if !response.status().is_success() {
                return Err(read_error_code(response).await);
            }
            let start = response
                .json::<AuthStartResponse>()
                .await
                .map_err(|_| "invalid_authorization_response".to_string())?;
            self.validate_authorization_url(&start.verification_uri, false)?;
            self.validate_authorization_url(&start.verification_uri_complete, true)?;
            Ok(start)
        })
    }

    fn poll<'a>(&'a self, device_code: &'a str) -> TransportFuture<'a, PollResponse> {
        Box::pin(async move {
            let response = self
                .http
                .post(self.endpoint("api/oauth/device/token")?)
                .json(&PollRequest {
                    client_id: CLIENT_ID,
                    device_code,
                })
                .send()
                .await
                .map_err(|_| "network_unavailable".to_string())?;
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                return Ok(PollResponse::SlowDown);
            }
            if response.status().is_success() {
                let credential = response
                    .json::<TokenResponse>()
                    .await
                    .map_err(|_| "invalid_authorization_response".to_string())?;
                return Ok(PollResponse::Approved(ManagedCredential {
                    credential: SecretCredential::new(credential.api_key),
                    base_url: credential.base_url,
                    user_email: credential.user_email,
                    key_name: credential.key_name,
                    group: credential.group,
                }));
            }
            match read_error_code(response).await.as_str() {
                "authorization_pending" => Ok(PollResponse::Pending),
                "slow_down" => Ok(PollResponse::SlowDown),
                "authorization_denied" | "access_denied" => Ok(PollResponse::Denied),
                "authorization_expired" | "expired_token" => Ok(PollResponse::Expired),
                "entitlement_required" => Ok(PollResponse::EntitlementMissing),
                "default_model_unavailable" => Ok(PollResponse::DefaultModelUnavailable),
                _ => Err("desktop_auth_protocol_error".to_string()),
            }
        })
    }

    fn discover<'a>(
        &'a self,
        credential: &'a SecretCredential,
        base_url: &'a str,
    ) -> TransportFuture<'a, DiscoveryResponse> {
        Box::pin(async move {
            if base_url != self.base_url {
                return Err("untrusted_base_url".to_string());
            }
            let response = self
                .http
                .get(self.endpoint("v1/dashboard/beefex/groups")?)
                .bearer_auth(credential.expose())
                .send()
                .await
                .map_err(|_| "network_unavailable".to_string())?;
            if response.status() == StatusCode::UNAUTHORIZED
                || response.status() == StatusCode::FORBIDDEN
            {
                return Err("reauthorization_required".to_string());
            }
            if !response.status().is_success() {
                return Err(read_error_code(response).await);
            }
            response
                .json::<DiscoveryResponse>()
                .await
                .map_err(|_| "invalid_discovery_response".to_string())
        })
    }

    fn revoke<'a>(
        &'a self,
        credential: &'a SecretCredential,
        base_url: &'a str,
    ) -> TransportFuture<'a, ()> {
        Box::pin(async move {
            if base_url != self.base_url {
                return Err("untrusted_base_url".to_string());
            }
            let response = self
                .http
                .delete(self.endpoint("v1/dashboard/beefex/token")?)
                .bearer_auth(credential.expose())
                .send()
                .await
                .map_err(|_| "network_unavailable".to_string())?;
            if response.status().is_success() || response.status() == StatusCode::UNAUTHORIZED {
                return Ok(());
            }
            Err("token_revoke_failed".to_string())
        })
    }

    fn ensure_client_credentials<'a>(
        &'a self,
        credential: &'a SecretCredential,
        base_url: &'a str,
    ) -> TransportFuture<'a, ManagedClientCredentials> {
        Box::pin(async move {
            if base_url != self.base_url {
                return Err("untrusted_base_url".to_string());
            }
            let response = self
                .http
                .post(self.endpoint("v1/dashboard/beefex/client-credentials")?)
                .bearer_auth(credential.expose())
                .send()
                .await
                .map_err(|_| "network_unavailable".to_string())?;
            if response.status() == StatusCode::UNAUTHORIZED
                || response.status() == StatusCode::FORBIDDEN
            {
                return Err("reauthorization_required".to_string());
            }
            if !response.status().is_success() {
                return Err(read_error_code(response).await);
            }
            let body = response
                .json::<ManagedClientCredentialsResponse>()
                .await
                .map_err(|_| "invalid_managed_client_credentials_response".to_string())?;
            if !body.success || body.base_url.trim().is_empty() {
                return Err("invalid_managed_client_credentials_response".to_string());
            }
            Ok(ManagedClientCredentials {
                origin: body.base_url,
                gpt: ManagedClientCredential {
                    credential: SecretCredential::new(body.credentials.gpt.key),
                    group: body.credentials.gpt.group,
                },
                claude: ManagedClientCredential {
                    credential: SecretCredential::new(body.credentials.claude.key),
                    group: body.credentials.claude.group,
                },
                grok: ManagedClientCredential {
                    credential: SecretCredential::new(body.credentials.grok.key),
                    group: body.credentials.grok.group,
                },
            })
        })
    }
}

#[derive(Serialize)]
struct StartRequest<'a> {
    client_id: &'static str,
    client_version: &'a str,
    hostname: &'a str,
    scope: &'static str,
    preferred_group: &'static str,
}

#[derive(Serialize)]
struct PollRequest<'a> {
    client_id: &'static str,
    device_code: &'a str,
}

#[derive(Deserialize)]
struct TokenResponse {
    api_key: String,
    base_url: String,
    user_email: String,
    key_name: String,
    group: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Deserialize)]
struct ManagedClientCredentialsResponse {
    success: bool,
    base_url: String,
    credentials: ManagedClientCredentialPair,
}

#[derive(Deserialize)]
struct ManagedClientCredentialPair {
    gpt: ManagedClientCredentialResponse,
    claude: ManagedClientCredentialResponse,
    grok: ManagedClientCredentialResponse,
}

#[derive(Deserialize)]
struct ManagedClientCredentialResponse {
    key: String,
    group: String,
}

async fn read_error_code(response: reqwest::Response) -> String {
    response
        .json::<ErrorResponse>()
        .await
        .map(|body| body.error)
        .unwrap_or_else(|_| "beefapi_request_failed".to_string())
}

fn configured_origin() -> Result<Url, String> {
    #[cfg(debug_assertions)]
    let candidate = std::env::var("BEEFEX_BEEFAPI_ORIGIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| PRODUCTION_ORIGIN.to_string());
    #[cfg(not(debug_assertions))]
    let candidate = PRODUCTION_ORIGIN.to_string();
    let parsed = Url::parse(candidate.trim()).map_err(|_| "beefapi_origin_invalid".to_string())?;
    if parsed.username() != ""
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || (parsed.path() != "" && parsed.path() != "/")
    {
        return Err("beefapi_origin_invalid".to_string());
    }
    if parsed.as_str().trim_end_matches('/') == PRODUCTION_ORIGIN {
        return Ok(parsed);
    }
    #[cfg(debug_assertions)]
    {
        let host = parsed.host_str().unwrap_or_default();
        if parsed.scheme() == "http" && (host == "127.0.0.1" || host == "localhost") {
            return Ok(parsed);
        }
    }
    Err("beefapi_origin_invalid".to_string())
}

pub(crate) fn production_parts(
    metadata_path: std::path::PathBuf,
) -> Result<
    (
        std::sync::Arc<dyn AccountTransport>,
        std::sync::Arc<dyn super::credential_store::CredentialStore>,
        std::sync::Arc<dyn super::account::AccountMetadataStore>,
    ),
    String,
> {
    let credential_path = metadata_path
        .parent()
        .ok_or_else(|| "credential_store_unavailable".to_string())?
        .join("credentials")
        .join("beefapi-managed");
    Ok((
        std::sync::Arc::new(BeefApiClient::new()?),
        std::sync::Arc::new(FileCredentialStore::new(credential_path)),
        std::sync::Arc::new(super::account::FileAccountMetadataStore::new(metadata_path)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_origin_and_base_are_canonical() {
        let client = BeefApiClient::new().unwrap();
        assert_eq!(
            client.trusted_base_url(),
            crate::beefapi::types::PRODUCTION_BASE_URL
        );
        assert_eq!(
            client.endpoint("api/oauth/device/code").unwrap().as_str(),
            "https://beefapi.com/api/oauth/device/code"
        );
    }
}

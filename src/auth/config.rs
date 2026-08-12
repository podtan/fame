use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub enabled: bool,
    pub issuer_url: String,
    pub expected_audience: Option<String>,
    pub dev_mode: bool,
    /// Optional explicit userinfo endpoint URL. If `None`, the URL is derived
    /// from the OIDC discovery document's `userinfo_endpoint` or `{issuer}/userinfo`.
    pub userinfo_url: Option<String>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            issuer_url: "http://localhost:8080".to_string(),
            expected_audience: None,
            dev_mode: false,
            userinfo_url: None,
        }
    }
}

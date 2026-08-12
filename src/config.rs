//! Configuration loaded via figment (TOML file + environment variables).
//!
//! Fame's config is loaded from `fame.toml` (or FAME_CONFIG_PATH) with
//! environment variable overrides prefixed with `FAME_`.

use serde::Deserialize;

use crate::auth::config::AuthConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub pdt_url: String,
    /// Entity config directory (where semantic.toml, episodic.toml, etc. live)
    #[serde(default = "default_entity_dir")]
    pub entity_dir: String,
    /// Authentication and OIDC configuration
    #[serde(default)]
    pub auth: AuthConfig,
    /// Cedar ABAC authorization configuration
    #[serde(default)]
    pub cedar: CedarConfig,
}

fn default_entity_dir() -> String {
    "entities".to_string()
}

/// Cedar authorization configuration for Fame
#[derive(Debug, Clone, Deserialize)]
pub struct CedarConfig {
    pub enabled: bool,
    pub policy_path: String,
    pub schema_path: String,
    pub validate_on_load: bool,
    /// Optional URL of a remote policy store endpoint (e.g. PDT `/api/cedar/policies`).
    #[serde(default)]
    pub policy_store_url: Option<String>,
    /// Optional Bearer token for the policy store endpoint.
    #[serde(default)]
    pub policy_store_token: Option<String>,
}

impl Default for CedarConfig {
    fn default() -> Self {
        Self {
            enabled: std::env::var("CEDAR_ENABLED")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
            policy_path: std::env::var("CEDAR_POLICY_PATH")
                .unwrap_or_else(|_| "./policies".to_string()),
            schema_path: std::env::var("CEDAR_SCHEMA_PATH")
                .unwrap_or_else(|_| "./policies/schema.cedarschema".to_string()),
            validate_on_load: std::env::var("CEDAR_VALIDATE_ON_LOAD")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
            policy_store_url: std::env::var("CEDAR_POLICY_STORE_URL").ok(),
            policy_store_token: std::env::var("CEDAR_POLICY_STORE_TOKEN").ok(),
        }
    }
}

impl From<CedarConfig> for pep::cedar::CedarConfig {
    fn from(config: CedarConfig) -> Self {
        Self {
            policy_path: std::path::PathBuf::from(&config.policy_path),
            schema_path: Some(std::path::PathBuf::from(&config.schema_path)),
            entities_path: None,
            default_decision: pep::cedar::config::DefaultDecision::Deny,
            validate_on_load: config.validate_on_load,
            policy_store_url: config.policy_store_url,
            policy_store_token: config.policy_store_token,
            embedded_policy: Some(include_str!("../policies/rbac.cedar")),
            embedded_schema: Some(include_str!("../policies/schema.cedarschema")),
        }
    }
}

impl Config {
    pub async fn load() -> anyhow::Result<Self> {
        // Try explicit config path first
        if let Ok(config_path) = std::env::var("FAME_CONFIG_PATH") {
            let toml_str = std::fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&toml_str)?;
            tracing::info!("Loaded config from local file: {}", config_path);
            return Ok(config);
        }

        // Fall back to figment (TOML + env)
        use figment::{
            providers::{Env, Format, Toml},
            Figment,
        };

        let config: Config = Figment::new()
            .merge(Toml::file("fame.toml"))
            .merge(Env::prefixed("FAME_").split("__"))
            .extract()
            .map_err(|e| anyhow::anyhow!("Failed to load configuration: {}", e))?;

        tracing::info!(
            "Loaded config: PDT={}, entity_dir={}",
            config.pdt_url,
            config.entity_dir
        );

        Ok(config)
    }
}

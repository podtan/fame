//! Fame — Facade for Agent Memory
//!
//! A config-driven entity server providing semantic, episodic, and procedural
//! memory types as PDT assets, scoped per-agent via X-Instance-Id header.
//!
//! Built on the same dynamic entity engine as NGHR. No hardcoded entity
//! handlers — all three memory types are defined as TOML configs and served
//! by the generic entity handler with agent_id tag injection for isolation.

mod auth;
mod cedar;
mod config;
mod entity_config;
mod entity_handlers;
mod models;
mod openapi_gen;
mod pdt;

use axum::{routing::get, Router};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{
    layer::SubscriberExt, util::SubscriberInitExt,
};
use utoipa_swagger_ui::SwaggerUi;
use pep::oidc_resource_server::ResourceServerClient;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<config::Config>,
    pub pdt: Arc<pdt::PdtClient>,
    pub authorizer: Option<Arc<pep::cedar::CedarAuthorizer>>,
    pub entity_configs: Vec<Arc<entity_config::EntityConfig>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::Config::load().await?;
    tracing::info!("Loaded config for Fame");

    // Load entity configs from the entities directory
    let entity_config_dir = std::env::var("FAME_ENTITY_DIR")
        .unwrap_or_else(|_| config.entity_dir.clone());
    let entity_configs_raw = entity_config::load_entity_configs(&entity_config_dir);
    let entity_configs: Vec<Arc<entity_config::EntityConfig>> =
        entity_configs_raw.into_iter().map(Arc::new).collect();
    tracing::info!(
        "Loaded {} entity configs from {}",
        entity_configs.len(),
        entity_config_dir
    );

    let pdt_client = pdt::PdtClient::new(&config.pdt_url);

    let state = AppState {
        config: Arc::new(config.clone()),
        pdt: Arc::new(pdt_client),
        authorizer: None,
        entity_configs: entity_configs.clone(),
    };

    // Initialize Cedar authorizer
    let authorizer = if config.cedar.enabled {
        let cedar_config: pep::cedar::CedarConfig = config.cedar.clone().into();
        match pep::cedar::CedarAuthorizer::new_with_policy_store(cedar_config).await {
            Ok(a) => {
                tracing::info!("Cedar authorizer initialized (embedded policies loaded)");
                Some(Arc::new(a))
            }
            Err(e) => {
                tracing::error!("Failed to init Cedar: {}. Running without.", e);
                None
            }
        }
    } else {
        tracing::info!("Cedar authorization disabled");
        None
    };

    // Initialize auth layer with ResourceServerClient
    let auth_client = ResourceServerClient::new();
    let auth_layer = auth::middleware::AuthLayer::new(config.auth.clone(), auth_client);
    tracing::info!("Auth layer initialized (enabled: {})", config.auth.enabled);

    let state = AppState {
        authorizer,
        ..state
    };

    // Public routes (no auth)
    let public_routes = Router::new().route("/health", get(health));

    // Protected routes (auth + Cedar)
    let mut protected_routes = Router::new()
        // Entity config discovery endpoint (for Torpi/Aether to discover available memory types)
        .route(
            "/api/v1/config/entities",
            get(entity_handlers::list_entity_configs),
        );

    // Register config-driven entity routes (semantic, episodic, procedural memories)
    protected_routes = entity_handlers::register_entity_routes(protected_routes, &entity_configs);

    // Apply auth layer to all protected routes
    let protected_routes = protected_routes.layer(auth_layer);

    // Generate merged OpenAPI spec (utoipa base + config-driven entity paths)
    let merged_openapi = openapi_gen::generate_merged_openapi(&entity_configs);

    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(
            SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", merged_openapi),
        )
        .with_state(state)
        .layer(CorsLayer::permissive());

    let addr = format!("{}:{}", config.host, config.port);
    tracing::info!("Fame starting on {}", addr);
    tracing::info!("Swagger UI: http://{}/swagger-ui", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> &'static str {
    "OK"
}

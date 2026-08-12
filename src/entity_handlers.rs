//! Generic entity CRUD handler
//!
//! A single set of handlers that serves all config-driven entities (documents,
//! etc.) by reading their [`EntityConfig`] definitions and mapping between PDT
//! assets and entity JSON.
//!
//! Routes are registered dynamically in `main.rs` via [`register_entity_routes`].

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use crate::auth::{AuthenticatedUser, ForwardedToken, InstanceContext};
use crate::entity_config::EntityConfig;
use crate::pdt::{CreateAssetRequest, CreateRelationRequest, CreateTagRequest, PdtAsset, PdtSearchResult};
use crate::AppState;

// ---------------------------------------------------------------------------
// Field mapping layer
// ---------------------------------------------------------------------------

/// Extract a field value from a PDT asset based on the field's `map` hint.
///
/// Supported mappings:
/// - `"title_suffix"` — strips the entity's `title_prefix` from the asset title
/// - `"content"`      — the asset's `content` string
/// - `"tag:X"`        — tag with category `X`
/// - `"metadata:X"`   — value from `metadata[X]`
/// - `"created_at"`   — asset `created_at` timestamp
/// - `"updated_at"`   — asset `updated_at` timestamp
fn extract_field_value(
    asset: &PdtAsset,
    field_name: &str,
    field_map: &Option<String>,
    config: &EntityConfig,
) -> Option<Value> {
    match field_map.as_deref() {
        Some("title_suffix") => {
            let title = asset.title.strip_prefix(&config.title_prefix).unwrap_or(&asset.title);
            if title.is_empty() {
                None
            } else {
                Some(Value::String(title.to_string()))
            }
        }
        Some("content") => {
            if asset.content.is_empty() {
                None
            } else {
                Some(Value::String(asset.content.clone()))
            }
        }
        Some(m) if m.starts_with("tag:") => {
            let tag_name = &m[4..];
            asset.get_tag(tag_name).map(|s| Value::String(s.to_string()))
        }
        Some(m) if m.starts_with("metadata:") => {
            let key = &m[9..];
            asset.metadata.get(key).cloned()
        }
        Some("created_at") => Some(Value::String(asset.created_at.clone())),
        Some("updated_at") => Some(Value::String(asset.updated_at.clone())),
        _ => {
            // No mapping hint — try tag with the field name as category
            asset.get_tag(field_name).map(|s| Value::String(s.to_string()))
        }
    }
}

/// Extract a field value from a compact PDT search result.
///
/// Same semantics as [`extract_field_value`] but for `PdtSearchResult` which
/// lacks `content` and `metadata`.
fn extract_field_value_compact(
    result: &PdtSearchResult,
    field_name: &str,
    field_map: &Option<String>,
    config: &EntityConfig,
) -> Option<Value> {
    match field_map.as_deref() {
        Some("title_suffix") => {
            let title = result.title.strip_prefix(&config.title_prefix).unwrap_or(&result.title);
            if title.is_empty() {
                None
            } else {
                Some(Value::String(title.to_string()))
            }
        }
        Some("content") => None, // Not available in compact results
        Some(m) if m.starts_with("tag:") => {
            let tag_name = &m[4..];
            result.get_tag(tag_name).map(|s| Value::String(s.to_string()))
        }
        Some(m) if m.starts_with("metadata:") => None, // Not available in compact results
        Some("created_at") | Some("updated_at") => Some(Value::String(result.updated_at.clone())),
        _ => result.get_tag(field_name).map(|s| Value::String(s.to_string())),
    }
}

/// Resolve relation fields for an asset by querying PDT for its relations.
///
/// For each relation field in the config, finds the related asset ID and adds
/// it to the entity JSON under the field name.
async fn resolve_relations(
    state: &AppState,
    instance_id: Option<&str>,
    asset_id: &str,
    config: &EntityConfig,
    entity: &mut serde_json::Map<String, Value>,
    token: Option<&str>,
) {
    let pdt = state.pdt.for_instance(instance_id);
    for (field_name, field_config) in config.relation_fields() {
        let rel_type = field_config
            .relation_type
            .as_deref()
            .unwrap_or("related_to");

        if let Ok(relations) = pdt.get_relations(asset_id, None, token).await {
            // Find the first relation matching the configured type where this
            // asset is the source (outgoing relation).
            let related_id = relations.iter().find_map(|r| {
                if r.from_asset_id == asset_id && r.relation_type == rel_type {
                    Some(r.to_asset_id.clone())
                } else {
                    None
                }
            });

            if let Some(id) = related_id {
                entity.insert(field_name.clone(), Value::String(id));
            }
        }
    }
}

/// Map a full PDT asset to an entity JSON object.
pub fn map_asset_to_entity(asset: &PdtAsset, config: &EntityConfig) -> Value {
    let mut entity = serde_json::Map::new();

    // Always include id
    entity.insert("id".to_string(), Value::String(asset.id.clone()));

    for (field_name, field_config) in &config.fields {
        if let Some(v) = extract_field_value(asset, field_name, &field_config.map, config) {
            entity.insert(field_name.clone(), v);
        }
    }

    Value::Object(entity)
}

/// Map a compact search result to an entity JSON object.
pub fn map_compact_to_entity(result: &PdtSearchResult, config: &EntityConfig) -> Value {
    let mut entity = serde_json::Map::new();

    entity.insert("id".to_string(), Value::String(result.id.clone()));

    for (field_name, field_config) in &config.fields {
        if let Some(v) = extract_field_value_compact(result, field_name, &field_config.map, config) {
            entity.insert(field_name.clone(), v);
        }
    }

    Value::Object(entity)
}

/// Map a full asset to entity JSON including resolved relations.
pub async fn map_asset_to_entity_with_relations(
    state: &AppState,
    instance_id: Option<&str>,
    asset: &PdtAsset,
    config: &EntityConfig,
    token: Option<&str>,
) -> Value {
    let mut entity_json = match map_asset_to_entity(asset, config) {
        Value::Object(m) => m,
        _ => return Value::Null,
    };

    resolve_relations(state, instance_id, &asset.id, config, &mut entity_json, token).await;

    Value::Object(entity_json)
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Generic query parameters — any key=value pair is accepted as a filter.
/// Known keys are extracted; the rest are passed through as tag filters.
#[derive(Debug, Deserialize, Clone)]
pub struct ListEntitiesQuery {
    /// Filter by status tag
    pub status: Option<String>,
    /// Filter by relation target (e.g. project_id for "related_to")
    pub attached_to: Option<String>,
}

// ---------------------------------------------------------------------------
// Handler context
// ---------------------------------------------------------------------------

/// Resolve entity config from the request path.
fn get_entity_config(state: &AppState, slug: &str) -> Option<Arc<EntityConfig>> {
    state
        .entity_configs
        .iter()
        .find(|c| c.slug == slug || c.slug == format!("{}s", slug))
        .cloned()
}

// ---------------------------------------------------------------------------
// Handlers (inner logic — slug is passed in, not extracted from Path)
// ---------------------------------------------------------------------------

/// GET /api/v1/{slug} — list entities
async fn list_entities_inner(
    state: &AppState,
    token: Option<&str>,
    instance_id: Option<&str>,
    slug: &str,
    query: &ListEntitiesQuery,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pdt = state.pdt.for_instance(instance_id);
    let config = get_entity_config(state, &slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Unknown entity type: {}", slug)})),
        )
    })?;

    let type_tag = config.type_tag().unwrap_or(&config.slug);

    // If attached_to filter is set, we need to find assets related to that ID
    let mut entities: Vec<Value> = if let Some(ref attached_to_id) = query.attached_to {
        // 1. Find all assets of this type
        let results = pdt.search_by_tag("type", type_tag, token)
            .await
            .unwrap_or_default();

        // Filter by agent_id (X-Instance-Id) for per-agent memory isolation
        let results: Vec<PdtSearchResult> = if let Some(ref agent_id) = instance_id {
            results.into_iter().filter(|r| r.get_tag("agent") == Some(agent_id)).collect()
        } else {
            results
        };

        // 2. For each result, check if it has a relation to attached_to_id
        let mut filtered = Vec::new();
        for r in &results {
            if let Ok(relations) = pdt.get_relations(&r.id, None, token).await {
                let has_relation = relations.iter().any(|rel| {
                    rel.from_asset_id == r.id
                        && rel.to_asset_id == *attached_to_id
                        && config
                            .relation_fields()
                            .iter()
                            .any(|(_, fc)| {
                                fc.relation_type.as_deref() == Some(rel.relation_type.as_str())
                            })
                });
                if has_relation {
                    let entity = map_compact_to_entity(r, &config);
                    filtered.push(entity);
                }
            }
        }
        filtered
    } else {
        let results = pdt.search_by_tag("type", type_tag, token)
            .await
            .unwrap_or_default();

        // Filter by agent_id (X-Instance-Id) for per-agent memory isolation
        let results: Vec<PdtSearchResult> = if let Some(ref agent_id) = instance_id {
            results.into_iter().filter(|r| r.get_tag("agent") == Some(agent_id)).collect()
        } else {
            results
        };

        // If this entity type has relation fields, skip orphaned entities
        // (those with no outgoing relations). A document that isn't linked
        // to any project/workstream/task/issue should not appear in NGHR.
        let has_relation_fields = !config.relation_fields().is_empty();

        let mut filtered: Vec<Value> = Vec::new();
        for r in &results {
            if has_relation_fields {
                let has_outgoing = match pdt.get_relations(&r.id, None, token).await {
                    Ok(relations) => relations.iter().any(|rel| rel.from_asset_id == r.id),
                    Err(_) => false,
                };
                if !has_outgoing {
                    continue;
                }
            }

            let entity = map_compact_to_entity(r, &config);

            // Apply status filter
            if let Some(ref status_filter) = query.status {
                let entity_status = entity.get("status").and_then(|s| s.as_str());
                if entity_status != Some(status_filter.as_str()) {
                    continue;
                }
            }

            filtered.push(entity);
        }
        filtered
    };

    // Sort by updated_at descending if that field exists
    entities.sort_by(|a, b| {
        let a_time = a.get("updated_at").and_then(|t| t.as_str()).unwrap_or("");
        let b_time = b.get("updated_at").and_then(|t| t.as_str()).unwrap_or("");
        b_time.cmp(a_time)
    });

    let total = entities.len();
    Ok(Json(json!({
        "entities": entities,
        "total": total,
    })))
}

/// GET /api/v1/{slug}/{id} — get a single entity by ID
async fn get_entity_inner(
    state: &AppState,
    token: Option<&str>,
    instance_id: Option<&str>,
    slug: &str,
    id: &str,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pdt = state.pdt.for_instance(instance_id);
    let config = get_entity_config(state, &slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Unknown entity type: {}", slug)})),
        )
    })?;

    let asset = pdt.get_asset(id, token)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    let entity = map_asset_to_entity_with_relations(state, instance_id, &asset, &config, token).await;

    Ok(Json(entity))
}

/// POST /api/v1/{slug} — create a new entity
async fn create_entity_inner(
    state: &AppState,
    user: &AuthenticatedUser,
    token: Option<&str>,
    instance_id: Option<&str>,
    slug: &str,
    body: Value,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let pdt = state.pdt.for_instance(instance_id);
    let config = get_entity_config(state, &slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Unknown entity type: {}", slug)})),
        )
    })?;

    // Cedar authorization check
    if let Some(ref authorizer) = state.authorizer {
        if let Some(ref action) = config.cedar.action_create {
            let claims = user.to_cedar_claims();
            crate::cedar::enforcement::check_permission(authorizer, &claims, action, &config.name, "<_>")
                .map_err(|status| (status, Json(json!({"error": "Access denied"}))))?;
        }
    }

    let body_obj = body.as_object().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Request body must be a JSON object"})),
        )
    })?;

    // Validate required fields
    for (field_name, field_config) in &config.fields {
        if field_config.required && !body_obj.contains_key(field_name) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Validation failed",
                    "details": format!("Missing required field: {}", field_name)
                })),
            ));
        }
    }

    // --- Build title ---
    let title_value = config
        .fields
        .iter()
        .find(|(_, fc)| fc.map.as_deref() == Some("title_suffix"))
        .and_then(|(name, _)| body_obj.get(name))
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled");

    let title = format!("{}{}", config.title_prefix, title_value);

    // --- Build content ---
    let content = config
        .fields
        .iter()
        .find(|(_, fc)| fc.map.as_deref() == Some("content"))
        .and_then(|(name, _)| body_obj.get(name))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // --- Build tags ---
    let mut tags: Vec<CreateTagRequest> = Vec::new();

    // Default tags from config
    for (category, value) in &config.default_tags {
        tags.push(CreateTagRequest {
            category: category.clone(),
            value: value.clone(),
        });
    }

    // Inject agent_id from X-Instance-Id as a tag for per-agent memory isolation.
    // The X-Instance-Id header carries the agent's identity (injected by Aether).
    // Fame uses it for dual purpose: PDT multi-tenant routing AND per-agent memory scoping.
    if let Some(ref agent_id) = instance_id {
        tags.push(CreateTagRequest {
            category: "agent".to_string(),
            value: agent_id.to_string(),
        });
    }

    // Fields mapped to tags
    for (field_name, field_config) in &config.fields {
        if let Some(ref m) = field_config.map {
            if let Some(tag_name) = m.strip_prefix("tag:") {
                // Use provided value, or fall back to default
                let value = body_obj
                    .get(field_name)
                    .and_then(|v| v.as_str())
                    .or(field_config.default.as_deref());

                if let Some(v) = value {
                    tags.push(CreateTagRequest {
                        category: tag_name.to_string(),
                        value: v.to_string(),
                    });
                }
            }
        }
    }

    // --- Build metadata ---
    let mut metadata: HashMap<String, Value> = HashMap::new();
    for (field_name, field_config) in &config.fields {
        if let Some(ref m) = field_config.map {
            if let Some(key) = m.strip_prefix("metadata:") {
                if let Some(v) = body_obj.get(field_name) {
                    metadata.insert(key.to_string(), v.clone());
                }
            }
        }
    }

    // --- Inherit auth_context from relation target (e.g. attached_to) ---
    let mut parent_auth_ctx = None;
    for (field_name, _field_config) in config.relation_fields() {
        if let Some(target_id) = body_obj.get(field_name).and_then(|v| v.as_str()) {
            parent_auth_ctx = pdt.get_auth_context(target_id, token).await.ok().flatten();
            if parent_auth_ctx.is_some() {
                break;
            }
        }
    }

    // --- Create the PDT asset ---
    let asset = pdt.create_asset(
            CreateAssetRequest {
                title,
                content,
                tags: Some(tags),
                auth_context: parent_auth_ctx,
            },
            token,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    // If metadata was provided, update the asset to include it
    if !metadata.is_empty() {
        let mut metadata_req = pdt.http_client()
            .put(format!("{}/api/assets/{}", pdt.base_url(), asset.id))
            .bearer_auth(token.unwrap_or(""));
        if let Some(id) = instance_id {
            metadata_req = metadata_req.header("X-Instance-Id", id);
        }
        let _ = metadata_req
            .json(&json!({"metadata": metadata}))
            .send()
            .await;
    }

    info!("Created {} entity: {} ({})", config.name, title_value, asset.id);

    // --- Create relations ---
    for (field_name, field_config) in config.relation_fields() {
        if let Some(target_id) = body_obj.get(field_name).and_then(|v| v.as_str()) {
            let rel_type = field_config
                .relation_type
                .as_deref()
                .unwrap_or("related_to");

            let _ = pdt
                .create_relation(
                    CreateRelationRequest {
                        from_asset_id: asset.id.clone(),
                        to_asset_id: target_id.to_string(),
                        relation_type: rel_type.to_string(),
                        metadata: None,
                    },
                    token,
                )
                .await;

            info!("Linked {} → {} ({})", asset.id, target_id, rel_type);
        }
    }

    // --- Build response ---
    let mut entity = map_asset_to_entity(&asset, &config);

    // Include relation fields in response
    if let Value::Object(ref mut map) = entity {
        for (field_name, _) in config.relation_fields() {
            if let Some(target_id) = body_obj.get(field_name).and_then(|v| v.as_str()) {
                map.insert(field_name.clone(), Value::String(target_id.to_string()));
            }
        }
    }

    Ok((StatusCode::CREATED, Json(entity)))
}

/// PATCH /api/v1/{slug}/{id}/status — update entity status
async fn update_entity_status_inner(
    state: &AppState,
    user: &AuthenticatedUser,
    token: Option<&str>,
    instance_id: Option<&str>,
    slug: &str,
    id: &str,
    body: Value,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let pdt = state.pdt.for_instance(instance_id);
    let config = get_entity_config(state, &slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Unknown entity type: {}", slug)})),
        )
    })?;

    // Cedar authorization check
    if let Some(ref authorizer) = state.authorizer {
        if let Some(ref action) = config.cedar.action_edit {
            let claims = user.to_cedar_claims();
            crate::cedar::enforcement::check_permission(authorizer, &claims, action, &config.name, id)
                .map_err(|status| (status, Json(json!({"error": "Access denied"}))))?;
        }
    }

    let new_status = body
        .get("status")
        .and_then(|s| s.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing 'status' field"})),
            )
        })?;

    // Validate status is in allowed values
    if let Some(allowed) = config.status_values() {
        if !allowed.iter().any(|v| v == new_status) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Invalid status",
                    "allowed_values": allowed,
                    "got": new_status
                })),
            ));
        }
    }

    let asset = pdt.update_asset_tag(id, "status", new_status, token)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    let entity = map_asset_to_entity_with_relations(state, instance_id, &asset, &config, token).await;

    Ok(Json(entity))
}

// ---------------------------------------------------------------------------
// Router builder
// ---------------------------------------------------------------------------

/// Register entity routes dynamically based on loaded configs.
///
/// For each entity config with `handler = "generic"`, registers routes:
///
/// ```text
/// GET    /api/v1/{slug}s            → list_entities
/// GET    /api/v1/{slug}s/{id}       → get_entity
/// POST   /api/v1/{slug}s            → create_entity
/// PATCH  /api/v1/{slug}s/{id}/status → update_entity_status
/// ```
///
/// Called from `main.rs` after configs are loaded.
pub fn register_entity_routes(
    router: axum::Router<AppState>,
    configs: &[Arc<EntityConfig>],
) -> axum::Router<AppState> {
    use axum::routing::{get, patch};

    let mut router = router;

    for config in configs {
        if config.handler != "generic" {
            continue;
        }

        // Pluralise slug for the path (e.g. "document" → "documents")
        let base = format!("/api/v1/{}s", config.slug);
        let slug = config.slug.clone();

        tracing::info!("Registering entity routes for {} at {}", config.name, base);

        // GET + POST /api/v1/{slug}s
        let slug_for_list = slug.clone();
        router = router.route(
            &base,
            get(move |State(state): State<AppState>,
                      ForwardedToken(token): ForwardedToken,
    instance: InstanceContext,
                      Query(query): Query<ListEntitiesQuery>| {
                let slug = slug_for_list.clone();
                async move {
                    list_entities_inner(&state, token.as_deref(), instance.as_deref(), &slug, &query).await
                }
            })
            .post(move |State(state): State<AppState>,
                        user: AuthenticatedUser,
                        ForwardedToken(token): ForwardedToken,
    instance: InstanceContext,
                        Json(body): Json<Value>| {
                let slug = slug.clone();
                async move {
                    create_entity_inner(&state, &user, token.as_deref(), instance.as_deref(), &slug, body).await
                }
            }),
        );

        // GET /api/v1/{slug}s/{id}
        let slug_for_get = config.slug.clone();
        router = router.route(
            &format!("{}/{{id}}", base),
            get(move |State(state): State<AppState>,
                      ForwardedToken(token): ForwardedToken,
    instance: InstanceContext,
                      Path(id): Path<String>| {
                let slug = slug_for_get.clone();
                async move {
                    get_entity_inner(&state, token.as_deref(), instance.as_deref(), &slug, &id).await
                }
            }),
        );

        // PATCH /api/v1/{slug}s/{id}/status
        let slug_for_patch = config.slug.clone();
        router = router.route(
            &format!("{}/{{id}}/status", base),
            patch(move |State(state): State<AppState>,
                        user: AuthenticatedUser,
                        ForwardedToken(token): ForwardedToken,
    instance: InstanceContext,
                        Path(id): Path<String>,
                        Json(body): Json<Value>| {
                let slug = slug_for_patch.clone();
                async move {
                    update_entity_status_inner(&state, &user, token.as_deref(), instance.as_deref(), &slug, &id, body).await
                }
            }),
        );
    }

    router
}

/// Serve available entity configs (for Torpi to discover).
pub async fn list_entity_configs(
    State(state): State<AppState>,
) -> Json<Value> {
    let configs: Vec<Value> = state
        .entity_configs
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "slug": c.slug,
                "title_prefix": c.title_prefix,
                "fields": c.fields.iter().map(|(name, fc)| {
                    json!({
                        "name": name,
                        "type": format!("{:?}", fc.field_type).to_lowercase(),
                        "required": fc.required,
                        "label": fc.label,
                        "values": fc.values,
                        "badge": fc.badge,
                        "in_views": fc.in_views,
                    })
                }).collect::<Vec<_>>(),
                "views": c.views.iter().map(|(_name, vc)| {
                    json!({
                        "layout": vc.layout,
                        "fields": vc.fields,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    Json(json!({
        "entities": configs,
        "total": configs.len(),
    }))
}

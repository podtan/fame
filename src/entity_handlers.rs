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

use crate::auth::{AuthenticatedUser, ForwardedToken, InstanceContext, WorkspaceAdmins};
use crate::entity_config::{slug_to_cedar_type, EntityConfig};
use crate::pdt::{
    AuthContext, CreateAssetRequest, CreateRelationRequest, CreateTagRequest, PdtAsset,
    PdtSearchResult,
};
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
            let title = asset
                .title
                .strip_prefix(&config.title_prefix)
                .unwrap_or(&asset.title);
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
            asset
                .get_tag(tag_name)
                .map(|s| Value::String(s.to_string()))
        }
        Some(m) if m.starts_with("metadata:") => {
            let key = &m[9..];
            asset.metadata.get(key).cloned()
        }
        Some("created_at") => Some(Value::String(asset.created_at.clone())),
        Some("updated_at") => Some(Value::String(asset.updated_at.clone())),
        _ => {
            // No mapping hint — try tag with the field name as category
            asset
                .get_tag(field_name)
                .map(|s| Value::String(s.to_string()))
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
            let title = result
                .title
                .strip_prefix(&config.title_prefix)
                .unwrap_or(&result.title);
            if title.is_empty() {
                None
            } else {
                Some(Value::String(title.to_string()))
            }
        }
        Some("content") => None, // Not available in compact results
        Some(m) if m.starts_with("tag:") => {
            let tag_name = &m[4..];
            result
                .get_tag(tag_name)
                .map(|s| Value::String(s.to_string()))
        }
        Some(m) if m.starts_with("metadata:") => None, // Not available in compact results
        Some("created_at") | Some("updated_at") => Some(Value::String(result.updated_at.clone())),
        _ => result
            .get_tag(field_name)
            .map(|s| Value::String(s.to_string())),
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
        if let Some(v) = extract_field_value_compact(result, field_name, &field_config.map, config)
        {
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

    resolve_relations(
        state,
        instance_id,
        &asset.id,
        config,
        &mut entity_json,
        token,
    )
    .await;

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

    // Entities sharing a type tag (the three memory types all use
    // `type=agent-memory`) are discriminated by their full default_tags —
    // apply the discriminator on every result, not just the type search.
    let matches_entity = |r: &PdtSearchResult| config.tags_match(&r.tags);

    // If attached_to filter is set, we need to find assets related to that ID
    let mut entities: Vec<Value> = if let Some(ref attached_to_id) = query.attached_to {
        // 1. Find all assets of this type
        let results = pdt
            .search_by_tag("type", type_tag, token)
            .await
            .unwrap_or_default();

        // Filter by agent_id (X-Instance-Id) for per-agent memory isolation
        let results: Vec<PdtSearchResult> = if let Some(ref agent_id) = instance_id {
            results
                .into_iter()
                .filter(|r| r.get_tag("agent") == Some(agent_id))
                .collect()
        } else {
            results
        };

        // 2. For each result, check if it has a relation to attached_to_id
        let mut filtered = Vec::new();
        for r in &results {
            if !matches_entity(r) {
                continue;
            }
            if let Ok(relations) = pdt.get_relations(&r.id, None, token).await {
                let has_relation = relations.iter().any(|rel| {
                    rel.from_asset_id == r.id
                        && rel.to_asset_id == *attached_to_id
                        && config.relation_fields().iter().any(|(_, fc)| {
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
        let results = pdt
            .search_by_tag("type", type_tag, token)
            .await
            .unwrap_or_default();

        // Filter by agent_id (X-Instance-Id) for per-agent memory isolation
        let results: Vec<PdtSearchResult> = if let Some(ref agent_id) = instance_id {
            results
                .into_iter()
                .filter(|r| r.get_tag("agent") == Some(agent_id))
                .collect()
        } else {
            results
        };

        // If this entity type has relation fields, skip orphaned entities
        // (those with no outgoing relations). A document that isn't linked
        // to any project/workstream/task/issue should not appear in NGHR.
        let has_relation_fields = !config.relation_fields().is_empty();

        let mut filtered: Vec<Value> = Vec::new();
        for r in &results {
            if !matches_entity(r) {
                continue;
            }
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

    let asset = pdt
        .get_asset(id, token)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, Json(json!({"error": e.to_string()}))))?;

    // Type guard: an asset requested via /semantic-memory/{id} must actually
    // BE a semantic memory. Without this, any asset ID from the same
    // instance resolves and gets mapped as the wrong entity type.
    if !config.tags_match(&asset.tags) {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("Asset {} is not a {}", id, config.name)
            })),
        ));
    }

    let entity =
        map_asset_to_entity_with_relations(state, instance_id, &asset, &config, token).await;

    Ok(Json(entity))
}

/// POST /api/v1/{slug} — create a new entity
/// Extract the groups claim (string array) from the authenticated user's
/// enriched claims.
fn user_groups(user: &AuthenticatedUser) -> Vec<String> {
    user.claims_extra
        .as_ref()
        .and_then(|c| c.get("groups"))
        .and_then(|g| g.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Pick the group that should OWN a memory created by `groups`:
/// the creator agent's per-agent memory group (`mem-*`) wins; otherwise the
/// workspace admin group (`ws-*-admins`, SPN `@domain` suffix tolerated).
/// None → caller stamps empty auth_context and logs loudly (admin-curable),
/// never rejects the write.
fn pick_owner_group(groups: &[String]) -> Option<String> {
    fn bare(g: &str) -> &str {
        g.split('@').next().unwrap_or(g)
    }
    groups
        .iter()
        .find(|g| bare(g).starts_with("mem-"))
        .cloned()
        .or_else(|| {
            groups
                .iter()
                .find(|g| {
                    let b = bare(g);
                    b.starts_with("ws-") && b.ends_with("-admins")
                })
                .cloned()
        })
}

async fn create_entity_inner(
    state: &AppState,
    user: &AuthenticatedUser,
    token: Option<&str>,
    instance_id: Option<&str>,
    slug: &str,
    body: Value,
    workspace_admins: WorkspaceAdmins,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let pdt = state.pdt.for_instance(instance_id);
    let config = get_entity_config(state, &slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Unknown entity type: {}", slug)})),
        )
    })?;

    // Cedar authorization check
    // NOTE: resource type must be the slug-derived Cedar identifier ("SemanticMemory"),
    // not config.name ("Semantic Memory") — spaces are illegal in Cedar type names
    // and would fail UID construction with a 500.
    //
    // Workspace scoping (0.2.0): when TOCPI creates an agent identity it
    // declares the workspace's admin group via X-Workspace-Admins; the scoped
    // check requires the caller's groups claim to contain it. Without the
    // header, only admin/conductor's unscoped permits can match. The value is
    // persisted into the asset metadata for later (trusted) edit-time checks.
    let mut workspace_admin_group: Option<String> = None;
    if let Some(ref authorizer) = state.authorizer {
        if let Some(ref action) = config.cedar.action_create {
            let claims = user.to_cedar_claims();
            let resource_attrs: Vec<(String, String)> = if slug == "agent-identity" {
                workspace_admin_group = workspace_admins.0.clone();
                workspace_admin_group
                    .clone()
                    .map(|g| {
                        vec![(
                            crate::cedar::enforcement::ADMIN_GROUP_METADATA_KEY.to_string(),
                            g,
                        )]
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            crate::cedar::enforcement::check_permission_scoped(
                authorizer,
                &claims,
                action,
                &slug_to_cedar_type(&config.slug),
                "<_>",
                &resource_attrs,
            )
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

    // Persist the workspace admin group on agent identities — the trusted
    // anchor for edit-time workspace scoping (header is create-time only).
    if let Some(group) = workspace_admin_group.as_ref() {
        metadata.insert(
            crate::cedar::enforcement::ADMIN_GROUP_METADATA_KEY.to_string(),
            Value::String(group.clone()),
        );
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

    // --- auth_context stamp (fallback when nothing is inherited) ---
    // Every memory must be born readable by its owner group: agent creators
    // stamp their per-agent `mem-<prefix>` group, human creators stamp their
    // workspace admin group. Empty auth_context = admin-only-invisible —
    // exactly the bug that made fresh agents blind to their own writes
    // (every memory needed a hand-patch until 2026-08-29). Groups are
    // stamped in the exact claim form so PDT's owner_groups matching sees
    // identical strings on both sides.
    let auth_context = parent_auth_ctx.or_else(|| {
        pick_owner_group(&user_groups(user)).map(|group| AuthContext {
            visibility: "team".to_string(),
            owner_groups: vec![group],
            confidentiality: String::new(),
        })
    });
    if auth_context.is_none() {
        tracing::warn!(
            "memory created with EMPTY auth_context: creator '{}' has no mem-* or ws-*-admins group — admin re-stamp required",
            user.user_id
        );
    }

    // --- Create the PDT asset ---
    let asset = pdt
        .create_asset(
            CreateAssetRequest {
                title,
                content,
                tags: Some(tags),
                auth_context,
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
        let mut metadata_req = pdt
            .http_client()
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

    info!(
        "Created {} entity: {} ({})",
        config.name, title_value, asset.id
    );

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

#[cfg(test)]
mod auth_context_stamp_tests {
    use super::pick_owner_group;

    #[test]
    fn agent_mem_group_wins() {
        let groups = vec![
            "pdt-api-agents".to_string(),
            "mem-e69605b1@idp.tanbal.ir".to_string(),
        ];
        assert_eq!(
            pick_owner_group(&groups).as_deref(),
            Some("mem-e69605b1@idp.tanbal.ir")
        );
    }

    #[test]
    fn human_ws_admins_group_when_no_mem_group() {
        let groups = vec![
            "pdt-api-users@idp.tanbal.ir".to_string(),
            "ws-9f2e7d01-4c5b-6a7d-8e9f-001122334455-admins".to_string(),
        ];
        assert_eq!(
            pick_owner_group(&groups).as_deref(),
            Some("ws-9f2e7d01-4c5b-6a7d-8e9f-001122334455-admins")
        );
    }

    #[test]
    fn spn_form_ws_admins_still_matches() {
        let groups = vec!["ws-abc123-admins@idp.tanbal.ir".to_string()];
        assert_eq!(
            pick_owner_group(&groups).as_deref(),
            Some("ws-abc123-admins@idp.tanbal.ir")
        );
    }

    #[test]
    fn plain_roles_never_own_memories() {
        let groups = vec![
            "pdt-api-users@idp.tanbal.ir".to_string(),
            "pdt-api-agents".to_string(),
        ];
        assert_eq!(pick_owner_group(&groups), None);
    }

    #[test]
    fn mem_group_preferred_over_ws_admins() {
        let groups = vec!["ws-abc-admins".to_string(), "mem-ff766ee2".to_string()];
        assert_eq!(pick_owner_group(&groups).as_deref(), Some("mem-ff766ee2"));
    }
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
    // Same slug-derived resource type as create (see note above).
    //
    // Workspace scoping (0.2.0): for agent identities the anchor comes from
    // the STORED asset metadata (admin_group), never from a caller header —
    // callers cannot edit identities they don't administer by declaring a
    // group of their own.
    if let Some(ref authorizer) = state.authorizer {
        if let Some(ref action) = config.cedar.action_edit {
            let claims = user.to_cedar_claims();
            let resource_attrs: Vec<(String, String)> = if slug == "agent-identity" {
                pdt.get_asset(id, token)
                    .await
                    .ok()
                    .and_then(|asset| {
                        asset
                            .metadata
                            .get(crate::cedar::enforcement::ADMIN_GROUP_METADATA_KEY)
                            .and_then(|v| v.as_str())
                            .map(|g| {
                                vec![(
                                    crate::cedar::enforcement::ADMIN_GROUP_METADATA_KEY.to_string(),
                                    g.to_string(),
                                )]
                            })
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            crate::cedar::enforcement::check_permission_scoped(
                authorizer,
                &claims,
                action,
                &slug_to_cedar_type(&config.slug),
                id,
                &resource_attrs,
            )
            .map_err(|status| (status, Json(json!({"error": "Access denied"}))))?;
        }
    }

    let new_status = body.get("status").and_then(|s| s.as_str()).ok_or_else(|| {
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

    let asset = pdt
        .update_asset_tag(id, "status", new_status, token)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
        })?;

    let entity =
        map_asset_to_entity_with_relations(state, instance_id, &asset, &config, token).await;

    Ok(Json(entity))
}

/// Update an agent identity's content (charter). PATCH /api/v1/{slug}s/{id}/content
///
/// Agent-identity only (v0.3.0) — the identity content update surface that
/// Cedar `EditAgentIdentity` was designed for but that never shipped. The
/// tocpi control plane (PATCH /api/agents/{id}) calls this so content lands
/// in place as the agent's live identity — update, never duplicate.
///
/// Semantics mirror tocpi v0.6.0 for consistency:
/// - `content` required; empty string = explicit wipe (logged loudly)
/// - response echoes old→new sha256 so callers verify what landed
/// - unknown id → loud 404, NEVER create-on-miss (provisioning is tocpi's
///   job via the create route)
/// - content only — title/status stay on their existing surfaces
///
/// Authorization reuses the wired workspace-scoped `EditAgentIdentity` path:
/// the anchor comes from the STORED asset metadata (admin_group), never from
/// a caller header.
///
/// Instance scoping is REQUIRED: identity assets live in the agent's nested
/// DB (created via X-Instance-Id routing). A request without the header
/// would silently look in the global DB — rejected with 400 instead.
async fn update_identity_content_inner(
    state: &AppState,
    user: &AuthenticatedUser,
    token: Option<&str>,
    instance_id: Option<&str>,
    slug: &str,
    id: &str,
    body: Value,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let instance_id = instance_id.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "X-Instance-Id header required: agent identity content                 updates are instance-scoped (the identity lives in the agent's nested DB)"})),
        )
    })?;
    let pdt = state.pdt.for_instance(Some(instance_id));
    let config = get_entity_config(state, slug).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Unknown entity type: {}", slug)})),
        )
    })?;

    // Loud 404 BEFORE authorization: an unknown id is a caller error, not an
    // access-control outcome, and the Cedar check needs the stored
    // admin_group from the asset anyway.
    let asset = pdt.get_asset(id, token).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Agent identity not found: {} ({})", id, e)})),
        )
    })?;

    // Cedar authorization check — same slug-derived resource type and
    // workspace scoping as the status update: the anchor comes from the
    // STORED asset metadata (admin_group), never from a caller header.
    if let Some(ref authorizer) = state.authorizer {
        if let Some(ref action) = config.cedar.action_edit {
            let claims = user.to_cedar_claims();
            let resource_attrs: Vec<(String, String)> = asset
                .metadata
                .get(crate::cedar::enforcement::ADMIN_GROUP_METADATA_KEY)
                .and_then(|v| v.as_str())
                .map(|g| {
                    vec![(
                        crate::cedar::enforcement::ADMIN_GROUP_METADATA_KEY.to_string(),
                        g.to_string(),
                    )]
                })
                .unwrap_or_default();
            crate::cedar::enforcement::check_permission_scoped(
                authorizer,
                &claims,
                action,
                &slug_to_cedar_type(&config.slug),
                id,
                &resource_attrs,
            )
            .map_err(|status| (status, Json(json!({"error": "Access denied"}))))?;
        }
    }

    // Content extraction: required, must be a string. Empty = explicit wipe.
    let new_content = body
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing or non-string 'content' field"})),
            )
        })?
        .to_string();
    if new_content.trim().is_empty() {
        tracing::warn!(
            "EXPLICIT CONTENT WIPE: user {} wiped content of agent identity {}",
            user.user_id,
            id
        );
    }

    // old→new sha256 echo (parity with tocpi v0.6.0)
    use sha2::Digest;
    let hash = |s: &str| format!("sha256:{:x}", sha2::Sha256::digest(s.as_bytes()));
    let old_hash = if asset.content.is_empty() {
        None
    } else {
        Some(hash(&asset.content))
    };

    let asset = pdt
        .update_asset_content(id, &new_content, token)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to update identity content: {}", e)})),
            )
        })?;

    tracing::info!(
        "Agent identity {} content updated by user {} ({} chars)",
        id,
        user.user_id,
        new_content.len()
    );

    let mut entity =
        map_asset_to_entity_with_relations(state, Some(instance_id), &asset, &config, token).await;
    if let Some(obj) = entity.as_object_mut() {
        obj.insert(
            "content_hash".to_string(),
            json!({
                "old": old_hash,
                "new": hash(&new_content),
            }),
        );
    }

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
            get(
                move |State(state): State<AppState>,
                      ForwardedToken(token): ForwardedToken,
                      instance: InstanceContext,
                      Query(query): Query<ListEntitiesQuery>| {
                    let slug = slug_for_list.clone();
                    async move {
                        list_entities_inner(
                            &state,
                            token.as_deref(),
                            instance.as_deref(),
                            &slug,
                            &query,
                        )
                        .await
                    }
                },
            )
            .post(
                move |State(state): State<AppState>,
                      user: AuthenticatedUser,
                      ForwardedToken(token): ForwardedToken,
                      instance: InstanceContext,
                      workspace_admins: WorkspaceAdmins,
                      Json(body): Json<Value>| {
                    let slug = slug.clone();
                    async move {
                        create_entity_inner(
                            &state,
                            &user,
                            token.as_deref(),
                            instance.as_deref(),
                            &slug,
                            body,
                            workspace_admins,
                        )
                        .await
                    }
                },
            ),
        );

        // GET /api/v1/{slug}s/{id}
        let slug_for_get = config.slug.clone();
        router = router.route(
            &format!("{}/{{id}}", base),
            get(
                move |State(state): State<AppState>,
                      ForwardedToken(token): ForwardedToken,
                      instance: InstanceContext,
                      Path(id): Path<String>| {
                    let slug = slug_for_get.clone();
                    async move {
                        get_entity_inner(&state, token.as_deref(), instance.as_deref(), &slug, &id)
                            .await
                    }
                },
            ),
        );

        // PATCH /api/v1/{slug}s/{id}/content — agent identity charters only
        // (v0.3.0): the governed identity-content write path. Scoped to
        // agent-identity on purpose; other entities keep their surfaces.
        if config.slug == "agent-identity" {
            let slug_for_content = config.slug.clone();
            router = router.route(
                &format!("{}/{{id}}/content", base),
                patch(
                    move |State(state): State<AppState>,
                          user: AuthenticatedUser,
                          ForwardedToken(token): ForwardedToken,
                          instance: InstanceContext,
                          Path(id): Path<String>,
                          Json(body): Json<Value>| {
                        let slug = slug_for_content.clone();
                        async move {
                            update_identity_content_inner(
                                &state,
                                &user,
                                token.as_deref(),
                                instance.as_deref(),
                                &slug,
                                &id,
                                body,
                            )
                            .await
                        }
                    },
                ),
            );
        }

        // PATCH /api/v1/{slug}s/{id}/status
        let slug_for_patch = config.slug.clone();
        router = router.route(
            &format!("{}/{{id}}/status", base),
            patch(
                move |State(state): State<AppState>,
                      user: AuthenticatedUser,
                      ForwardedToken(token): ForwardedToken,
                      instance: InstanceContext,
                      Path(id): Path<String>,
                      Json(body): Json<Value>| {
                    let slug = slug_for_patch.clone();
                    async move {
                        update_entity_status_inner(
                            &state,
                            &user,
                            token.as_deref(),
                            instance.as_deref(),
                            &slug,
                            &id,
                            body,
                        )
                        .await
                    }
                },
            ),
        );
    }

    router
}

/// Serve available entity configs (for Torpi to discover).
pub async fn list_entity_configs(State(state): State<AppState>) -> Json<Value> {
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

// ---------------------------------------------------------------------------
// Tests — update_identity_content_inner: mock PDT + real Cedar policies
// ---------------------------------------------------------------------------

#[cfg(test)]
mod identity_content_tests {
    //! Integration tests for update_identity_content_inner.
    //!
    //! TWO BACKEND TIERS, selected automatically:
    //! - **real-pdt**: a real PDT server (dev auth, sqlite backend) booted
    //!   from a PREBUILT sibling binary (`../pdt/target/debug/pdt`, or
    //!   `FAME_IT_PDT_BIN`). Full tenant routing — nested-DB provisioning,
    //!   X-Instance-Id isolation, real wire.
    //! - **mock-pdt** (always available, CI default): an in-process axum
    //!   server speaking PDT's wire format (tag objects carry ids, assets
    //!   use `_id`) — keeps the gate green without a sibling checkout.
    //!
    //! The tier is announced on stdout; tests are identical for both.
    //! NOTE: never build pdt on demand inside tests — a missing binary
    //! silently downgrades to the mock tier (loud eprintln).

    use super::*;
    use crate::pdt::{CreateAssetRequest, CreateTagRequest, PdtClient};
    use axum::response::IntoResponse;
    use axum::routing::{get, post, put};
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    const ADMIN_GROUP: &str = "ws-1-admins";
    const OLD: &str = "old charter";
    const NEW: &str = "new charter";

    fn user_with(role: &str, groups: &[&str]) -> AuthenticatedUser {
        AuthenticatedUser {
            user_id: "user-test".to_string(),
            username: Some("tester".to_string()),
            email: None,
            claims_extra: Some(std::collections::HashMap::from([
                (
                    "role".to_string(),
                    serde_json::Value::String(role.to_string()),
                ),
                (
                    "groups".to_string(),
                    serde_json::Value::Array(
                        groups
                            .iter()
                            .map(|g| serde_json::Value::String(g.to_string()))
                            .collect(),
                    ),
                ),
            ])),
        }
    }

    fn sha_hex(s: &str) -> String {
        use sha2::Digest;
        format!("sha256:{:x}", sha2::Sha256::digest(s.as_bytes()))
    }

    // ------------------------------------------------------------------
    // Tier selection
    // ------------------------------------------------------------------

    fn prebuilt_pdt_bin() -> Option<std::path::PathBuf> {
        if let Ok(p) = std::env::var("FAME_IT_PDT_BIN") {
            let b = std::path::PathBuf::from(p);
            if b.exists() {
                return Some(b);
            }
        }
        let b = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()?
            .join("pdt/target/debug/pdt");
        b.exists().then_some(b)
    }

    struct PdtProc(Child);
    impl Drop for PdtProc {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    async fn free_port() -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    async fn wait_healthy(base: &str) {
        let http = reqwest::Client::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if tokio::time::Instant::now() > deadline {
                panic!("pdt did not become healthy at {base}");
            }
            if let Ok(resp) = http.get(format!("{base}/health")).send().await {
                if resp.status().is_success() {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    async fn spawn_real_pdt(bin: std::path::PathBuf) -> (String, PdtProc) {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let tmp = std::env::temp_dir().join(format!("fame-pdt-it-{}-{n}", std::process::id()));
        std::fs::create_dir_all(tmp.join("instances")).expect("tmp instances dir");
        let port = free_port().await;
        let child = Command::new(&bin)
            .env("PDT_HOST", "127.0.0.1")
            .env("PDT_PORT", port.to_string())
            .env("PDT_DB_BACKEND", "sqlite")
            .env("SQLITE_PATH", tmp.join("global.db"))
            .env("PDT_INSTANCES_DIR", tmp.join("instances"))
            .env("AUTH_ENABLED", "false")
            .env("AUTH_DEV_MODE", "true")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn prebuilt pdt binary");
        let base = format!("http://127.0.0.1:{port}");
        wait_healthy(&base).await;
        (base, PdtProc(child))
    }

    // ------------------------------------------------------------------
    // Mock tier — in-process, wire-accurate PDT stand-in
    // ------------------------------------------------------------------
    // Wire lessons baked in (learned against the real server):
    // - asset objects carry "_id" (fame PdtAsset renames id)
    // - TAG objects carry "id" + "category" + "value" (fame PdtTag requires id)
    // - /api/search returns { "data": [ ... ] }
    // - PUT merges partial JSON (content/title/metadata)

    #[derive(Default)]
    struct MockDb {
        assets: Vec<serde_json::Value>,
        next_id: u32,
    }

    struct MockState {
        db: Mutex<MockDb>,
    }

    fn mock_tag(db: &mut MockDb, category: &str, value: &str) -> serde_json::Value {
        db.next_id += 1;
        serde_json::json!({
            "id": format!("tag-{}", db.next_id),
            "category": category,
            "value": value,
            "added_by": "mock",
            "added_at": "2026-08-29T00:00:00Z"
        })
    }

    async fn spawn_mock_pdt() -> String {
        let state = std::sync::Arc::new(MockState {
            db: Mutex::new(MockDb::default()),
        });

        // POST /api/assets — create
        let st = state.clone();
        let create_route = post(
            move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                let st = st.clone();
                async move {
                    let mut db = st.db.lock().unwrap();
                    db.next_id += 1;
                    let id = format!("ident-{:04}", db.next_id);
                    let mut tags: Vec<serde_json::Value> = Vec::new();
                    if let Some(list) = body.get("tags").and_then(|t| t.as_array()) {
                        for t in list {
                            tags.push(mock_tag(
                                &mut db,
                                t["category"].as_str().unwrap_or_default(),
                                t["value"].as_str().unwrap_or_default(),
                            ));
                        }
                    }
                    let _ = headers; // instance routing is process-global in the mock
                    let asset = serde_json::json!({
                        "_id": id,
                        "title": body.get("title").cloned().unwrap_or(serde_json::Value::Null),
                        "content": body.get("content").cloned().unwrap_or(serde_json::Value::String(String::new())),
                        "tags": tags,
                        "metadata": {},
                        "created_at": "2026-08-29T00:00:00Z",
                        "updated_at": "2026-08-29T00:00:00Z"
                    });
                    db.assets.push(asset.clone());
                    axum::Json(asset).into_response()
                }
            },
        );

        // GET/PUT /api/assets/{id}
        let st_get = state.clone();
        let st_put = state.clone();
        let asset_route = get(move |Path(id): Path<String>| {
            let st = st_get.clone();
            async move {
                use axum::response::IntoResponse;
                let db = st.db.lock().unwrap();
                match db.assets.iter().find(|a| a["_id"] == id) {
                    Some(a) => axum::Json(a.clone()).into_response(),
                    None => (
                        axum::http::StatusCode::NOT_FOUND,
                        axum::Json(serde_json::json!({"error": "not found"})),
                    )
                        .into_response(),
                }
            }
        })
        .put(
            move |Path(id): Path<String>, Json(body): Json<serde_json::Value>| {
                let st = st_put.clone();
                async move {
                    use axum::response::IntoResponse;
                    let mut db = st.db.lock().unwrap();
                    match db.assets.iter_mut().find(|a| a["_id"] == id) {
                        Some(a) => {
                            if let Some(c) = body.get("content") {
                                a["content"] = c.clone();
                            }
                            if let Some(t) = body.get("title") {
                                a["title"] = t.clone();
                            }
                            if let Some(m) = body.get("metadata").and_then(|m| m.as_object()) {
                                let md = a["metadata"].as_object_mut().unwrap();
                                for (k, v) in m {
                                    md.insert(k.clone(), v.clone());
                                }
                            }
                            a["updated_at"] =
                                serde_json::Value::String("2026-08-31T12:00:00Z".into());
                            axum::Json(a.clone()).into_response()
                        }
                        None => (
                            axum::http::StatusCode::NOT_FOUND,
                            axum::Json(serde_json::json!({"error": "not found"})),
                        )
                            .into_response(),
                    }
                }
            },
        );

        // GET /api/assets/{id}/relations
        let relations_route =
            get(move |Path(_id): Path<String>| async move { axum::Json(serde_json::json!([])) });

        // GET /api/search — compact results, { data: [...] }
        let st = state.clone();
        let search_route = get(
            move |Query(q): Query<std::collections::HashMap<String, String>>| {
                let st = st.clone();
                async move {
                    use axum::response::IntoResponse;
                    let tag = q.get("tag").cloned().unwrap_or_default();
                    let (cat, val) = match tag.split_once(':') {
                        Some((c, v)) => (c.to_string(), v.to_string()),
                        None => (tag.clone(), String::new()),
                    };
                    let db = st.db.lock().unwrap();
                    let data: Vec<serde_json::Value> = db
                        .assets
                        .iter()
                        .filter(|a| {
                            a["tags"].as_array().is_some_and(|tags| {
                                tags.iter().any(|t| {
                                    t["category"] == serde_json::Value::String(cat.clone())
                                        && t["value"] == serde_json::Value::String(val.clone())
                                })
                            })
                        })
                        .map(|a| {
                            serde_json::json!({
                                "_id": a["_id"],
                                "title": a["title"],
                                "tags": a["tags"],
                                "updated_at": a["updated_at"]
                            })
                        })
                        .collect();
                    axum::Json(serde_json::json!({ "data": data })).into_response()
                }
            },
        );

        let app = axum::Router::new()
            .route("/api/assets", create_route)
            .route("/api/assets/{id}", asset_route)
            .route("/api/assets/{id}/relations", relations_route)
            .route("/api/search", search_route);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    use axum::extract::Query;

    static SEQ: AtomicU32 = AtomicU32::new(0);

    // ------------------------------------------------------------------
    // Shared environment
    // ------------------------------------------------------------------

    struct TestEnv {
        state: AppState,
        base: String,
        ws: String,
        agent: String,
        identity_id: String,
        _proc: Option<PdtProc>,
    }

    async fn test_env() -> TestEnv {
        let (tier, base, proc) = match prebuilt_pdt_bin() {
            Some(bin) => {
                let (base, proc) = spawn_real_pdt(bin).await;
                ("real-pdt", base, Some(proc))
            }
            None => {
                eprintln!(
                    "identity_content_tests: backend = mock-pdt (prebuilt pdt binary not found; \
                     build ../pdt with `cargo build --no-default-features --features \
                     sqlite-backend` to run the real tier)"
                );
                ("mock-pdt", spawn_mock_pdt().await, None)
            }
        };
        let _ = tier;

        let http = reqwest::Client::new();

        // Provisioning (real tier only — the mock has no tenant routing;
        // per-test processes keep mock-tier tests isolated).
        let ws = uuid::Uuid::new_v4().to_string();
        let agent = uuid::Uuid::new_v4().to_string();
        if proc.is_some() {
            for (id, parent) in [(&ws, None), (&agent, Some(&ws))] {
                let resp = http
                    .post(format!("{base}/api/instances/{id}/provision"))
                    .query(&[("parent", parent.map(|p| p.to_string()))])
                    .send()
                    .await
                    .expect("provision request");
                assert!(
                    resp.status().is_success(),
                    "provision {id} failed: {}",
                    resp.status()
                );
            }
        }

        // Real Cedar authorizer over the same embedded policies prod uses.
        let cedar_config: pep::cedar::CedarConfig = crate::config::CedarConfig {
            enabled: true,
            policy_path: "./policies".to_string(),
            schema_path: "./policies/schema.cedarschema".to_string(),
            validate_on_load: true,
            policy_store_url: None,
            policy_store_token: None,
        }
        .into();
        let authorizer = pep::cedar::CedarAuthorizer::new_with_policy_store(cedar_config)
            .await
            .expect("authorizer loads from embedded policies");

        // Real entity configs from the repo's entities/ directory.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("entities");
        let entity_configs: Vec<_> =
            crate::entity_config::load_entity_configs(dir.to_str().unwrap())
                .into_iter()
                .map(Arc::new)
                .collect();
        assert!(entity_configs.iter().any(|c| c.slug == "agent-identity"));

        let config: crate::config::Config =
            toml::from_str(&format!("host = '127.0.0.1'\nport = 0\npdt_url = '{base}'"))
                .expect("minimal test config parses");
        let state = AppState {
            config: Arc::new(config),
            pdt: Arc::new(PdtClient::new(&base)),
            authorizer: Some(Arc::new(authorizer)),
            entity_configs,
        };

        // Seed the identity asset THROUGH fame's own client (same wire as
        // prod), then persist the workspace admin_group anchor the way the
        // create flow does (raw PUT with X-Instance-Id).
        let identity = state
            .pdt
            .for_instance(Some(&agent))
            .create_asset(
                CreateAssetRequest {
                    title: "Identity: Test Agent".to_string(),
                    content: Some(OLD.to_string()),
                    tags: Some(vec![
                        CreateTagRequest {
                            category: "type".to_string(),
                            value: "agent-identity".to_string(),
                        },
                        CreateTagRequest {
                            category: "agent".to_string(),
                            value: agent.clone(),
                        },
                    ]),
                    auth_context: None,
                },
                None,
            )
            .await
            .expect("seed identity via pdt client");
        let resp = http
            .put(format!("{base}/api/assets/{}", identity.id))
            .header("X-Instance-Id", &agent)
            .json(&serde_json::json!({ "metadata": { "admin_group": ADMIN_GROUP } }))
            .send()
            .await
            .expect("metadata patch");
        assert!(resp.status().is_success(), "metadata patch failed");

        TestEnv {
            state,
            base,
            ws,
            agent,
            identity_id: identity.id,
            _proc: proc,
        }
    }

    fn body(content: &str) -> Value {
        serde_json::json!({ "content": content })
    }

    async fn call(
        env: &TestEnv,
        user: &AuthenticatedUser,
        instance: Option<&str>,
        id: &str,
        body: Value,
    ) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
        update_identity_content_inner(&env.state, user, None, instance, "agent-identity", id, body)
            .await
    }

    /// Roundtrip: content lands in place, hashes echo, exactly ONE identity
    /// record exists (no duplicates).
    #[tokio::test]
    async fn roundtrip_updates_in_place_with_hash_echo() {
        let env = test_env().await;
        let resp = call(
            &env,
            &user_with("user", &[ADMIN_GROUP]),
            Some(&env.agent),
            &env.identity_id,
            body(NEW),
        )
        .await
        .expect("scoped user may edit identity content");

        let resp = resp.0;
        assert_eq!(resp["content"], NEW, "content lands");
        assert_eq!(resp["content_hash"]["old"], sha_hex(OLD), "old hash echoes");
        assert_eq!(resp["content_hash"]["new"], sha_hex(NEW), "new hash echoes");

        let inst = env.state.pdt.for_instance(Some(&env.agent));
        let read = inst
            .get_asset(&env.identity_id, None)
            .await
            .expect("identity readable");
        assert_eq!(read.content, NEW);
        let idents = inst
            .search_by_tag("type", "agent-identity", None)
            .await
            .expect("identity search");
        assert_eq!(idents.len(), 1, "zero duplicate identity records");
    }

    /// Unknown identity → loud 404, never create-on-miss.
    #[tokio::test]
    async fn unknown_identity_is_loud_404() {
        let env = test_env().await;
        let err = call(
            &env,
            &user_with("admin", &[]),
            Some(&env.agent),
            "ident-missing",
            body(NEW),
        )
        .await
        .expect_err("unknown id must 404");
        assert_eq!(err.0, StatusCode::NOT_FOUND);
        assert!(
            err.1 .0["error"]
                .as_str()
                .unwrap()
                .contains("Agent identity not found"),
            "loud message: {}",
            err.1 .0
        );
    }

    /// Cedar deny: principal outside the workspace admin_group cannot edit.
    #[tokio::test]
    async fn cedar_denies_principal_outside_admin_group() {
        let env = test_env().await;
        let err = call(
            &env,
            &user_with("user", &["some-other-group"]),
            Some(&env.agent),
            &env.identity_id,
            body(NEW),
        )
        .await
        .expect_err("out-of-group user must be denied");
        assert_eq!(err.0, StatusCode::FORBIDDEN);
        assert_eq!(err.1 .0["error"], "Access denied");
        let asset = env
            .state
            .pdt
            .for_instance(Some(&env.agent))
            .get_asset(&env.identity_id, None)
            .await
            .unwrap();
        assert_eq!(asset.content, OLD, "denied write must not land");
    }

    /// Admin role bypasses group scoping (policy: admin permits all).
    #[tokio::test]
    async fn admin_role_edits_without_group() {
        let env = test_env().await;
        let resp = call(
            &env,
            &user_with("admin", &[]),
            Some(&env.agent),
            &env.identity_id,
            body(NEW),
        )
        .await
        .expect("admin may edit");
        assert_eq!(resp.0["content"], NEW);
    }

    /// Agent role can edit its own identity (own nested DB via instance header).
    #[tokio::test]
    async fn agent_role_edits_own_identity() {
        let env = test_env().await;
        let resp = call(
            &env,
            &user_with("agent", &[]),
            Some(&env.agent),
            &env.identity_id,
            body("self-written charter"),
        )
        .await
        .expect("agent may edit own identity");
        assert_eq!(resp.0["content"], "self-written charter");
    }

    /// Empty content = explicit wipe: allowed, stored empty. The entity
    /// mapper omits empty fields, so verify via hash echo + stored asset.
    #[tokio::test]
    async fn empty_content_is_explicit_wipe() {
        let env = test_env().await;
        let resp = call(
            &env,
            &user_with("user", &[ADMIN_GROUP]),
            Some(&env.agent),
            &env.identity_id,
            body(""),
        )
        .await
        .expect("explicit wipe allowed");
        assert_eq!(resp.0["content_hash"]["new"], sha_hex(""));
        let asset = env
            .state
            .pdt
            .for_instance(Some(&env.agent))
            .get_asset(&env.identity_id, None)
            .await
            .unwrap();
        assert_eq!(asset.content, "", "wipe stored as empty string");
    }

    /// Missing instance header → loud 400: identity content is
    /// instance-scoped; a global-DB lookup would silently miss.
    #[tokio::test]
    async fn missing_instance_header_is_loud_400() {
        let env = test_env().await;
        let err = call(
            &env,
            &user_with("admin", &[]),
            None,
            &env.identity_id,
            body(NEW),
        )
        .await
        .expect_err("missing instance header must 400");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(
            err.1 .0["error"]
                .as_str()
                .unwrap()
                .contains("X-Instance-Id header required"),
            "loud message: {}",
            err.1 .0
        );
    }

    /// Manager-agent bootstrap: role=agent may CREATE a missing identity in
    /// a report agent's home instance (charter provisioning, fame 0.3.2).
    /// This is the exact Farzan→Saman/Ravand/Bonyan charter flow.
    #[tokio::test]
    async fn agent_role_creates_missing_identity_allowed() {
        let env = test_env().await;
        let report_instance = uuid::Uuid::new_v4().to_string();
        let new_id = uuid::Uuid::new_v4().to_string();
        let resp = call(
            &env,
            &user_with("agent", &[]),
            Some(&report_instance),
            &new_id,
            body(NEW),
        )
        .await
        .expect("manager agent may bootstrap a report identity");
        assert_eq!(resp.0["content"], NEW);
    }

    /// Tocpi's service principal may bootstrap identity content too
    /// (control-plane provisioning on behalf of the platform).
    #[tokio::test]
    async fn service_role_creates_missing_identity_allowed() {
        let env = test_env().await;
        let report_instance = uuid::Uuid::new_v4().to_string();
        let new_id = uuid::Uuid::new_v4().to_string();
        let resp = call(
            &env,
            &user_with("service", &[]),
            Some(&report_instance),
            &new_id,
            body(NEW),
        )
        .await
        .expect("service principal may bootstrap identity");
        assert_eq!(resp.0["content"], NEW);
    }

}

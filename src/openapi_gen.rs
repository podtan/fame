//! Runtime OpenAPI generation for config-driven entities
//!
//! Generates OpenAPI paths and JSON schemas from [`EntityConfig`] definitions,
//! merges them with the existing utoipa-generated spec, and serves the combined
//! result so Aether (and other consumers) can discover document endpoints.

use std::sync::Arc;

use serde_json::{json, Map, Value};
use utoipa::openapi::OpenApi;
use utoipa::OpenApi as OpenApiDerive;

use crate::entity_config::{EntityConfig, FieldType};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity_config::EntityFile;

    /// The served spec must carry the list-bound surface: limit + verbose
    /// params on every config-driven list route, `returned` in every
    /// ListResponse schema. Also dumps docs/openapi.json — the versioned
    /// artifact Paydar syncs into the aether configs (both copies) at
    /// deploy; a code change that moves the spec shows up as this file's
    /// diff in review.
    #[test]
    fn generated_spec_carries_list_bound_surface() {
        let configs: Vec<Arc<EntityConfig>> = [
            include_str!("../entities/episodic.toml"),
            include_str!("../entities/procedural.toml"),
            include_str!("../entities/semantic.toml"),
        ]
        .iter()
        .map(|src| {
            let file: EntityFile = toml::from_str(src).unwrap();
            Arc::new(file.entity)
        })
        .collect();

        let api = generate_merged_openapi(&configs);
        let spec: Value = serde_json::to_value(&api).unwrap();

        // The three memory list routes exist and carry limit + verbose.
        for slug in ["episodic-memorys", "procedural-memorys", "semantic-memorys"] {
            let path = format!("/api/v1/{slug}");
            let get = spec["paths"]
                .get(&path)
                .and_then(|p| p.get("get"))
                .unwrap_or_else(|| panic!("{path}: list route missing from spec"));
            let params = get["parameters"].as_array().unwrap();
            let names: Vec<&str> = params.iter().filter_map(|p| p["name"].as_str()).collect();
            assert!(names.contains(&"limit"), "{path}: limit param missing");
            assert!(names.contains(&"verbose"), "{path}: verbose param missing");
        }

        // Every generated ListResponse schema carries returned.
        let schemas = spec["components"]["schemas"].as_object().unwrap();
        let list_schemas: Vec<(&String, &Value)> = schemas
            .iter()
            .filter(|(_, s)| {
                s.get("properties")
                    .and_then(|p| p.get("entities"))
                    .is_some()
            })
            .collect();
        assert_eq!(
            list_schemas.len(),
            3,
            "three ListResponse schemas expected (one per memory family)"
        );
        for (name, s) in list_schemas {
            assert!(
                s["properties"].get("returned").is_some(),
                "{name}: ListResponse schema missing returned"
            );
        }

        // Dump the versioned artifact for the aether sync.
        std::fs::create_dir_all("docs").unwrap();
        std::fs::write(
            "docs/openapi.json",
            serde_json::to_string_pretty(&spec).unwrap(),
        )
        .unwrap();
    }
}

/// Generate a merged OpenAPI spec: the existing utoipa [`ApiDoc`] paths/schemas
/// plus dynamically generated paths/schemas for all config-driven entities.
///
/// Uses JSON manipulation on the serialised utoipa spec for simplicity and
/// reliability across utoipa versions.
pub fn generate_merged_openapi(configs: &[Arc<EntityConfig>]) -> OpenApi {
    // Get the base spec from utoipa
    let base_api = crate::models::ApiDoc::openapi();

    // Serialize to JSON for manipulation
    let mut spec: Value = serde_json::to_value(&base_api).unwrap_or_else(|_| {
        json!({
            "openapi": "3.0.3",
            "info": {"title": "NGHR", "version": "0.1.0"},
            "paths": {},
            "components": {"schemas": {}},
        })
    });

    // Ensure paths and components exist
    if spec.get("paths").is_none() {
        spec["paths"] = json!({});
    }
    if spec.get("components").is_none() {
        spec["components"] = json!({});
    }
    if spec["components"].get("schemas").is_none() {
        spec["components"]["schemas"] = json!({});
    }

    for config in configs {
        if config.handler != "generic" {
            continue;
        }

        let slug_plural = format!("{}s", config.slug);
        let base_path = format!("/api/v1/{}", slug_plural);
        let entity_name = &config.name;

        // --- Add schemas ---
        let schemas = &mut spec["components"]["schemas"];

        // Entity response schema
        schemas[entity_name] = generate_entity_schema_json(config);

        // Create request schema
        schemas[&format!("Create{}Request", entity_name)] = generate_create_schema_json(config);

        // List response schema
        schemas[&format!("{}ListResponse", entity_name)] = json!({
            "type": "object",
            "properties": {
                "entities": {
                    "type": "array",
                    "items": {
                        "$ref": format!("#/components/schemas/{}", entity_name)
                    }
                },
                "total": {"type": "integer"},
                "returned": {"type": "integer"}
            },
            "required": ["entities", "total", "returned"]
        });

        // --- Add paths ---
        let paths = &mut spec["paths"];

        // GET /{base} — list
        let mut list_params = vec![
            json!({
                "name": "status",
                "in": "query",
                "required": false,
                "schema": {"type": "string"},
                "description": "Filter by status"
            }),
            json!({
                "name": "attached_to",
                "in": "query",
                "required": false,
                "schema": {"type": "string"},
                "description": "Filter by relation target (e.g. project ID)"
            }),
            json!({
                "name": "limit",
                "in": "query",
                "required": false,
                "schema": {"type": "integer", "minimum": 0},
                "description": "Max rows returned. DEFAULT 20 (newest-first by updated_at) — a naive list call can never flood a context. Response always carries the true total + returned. Pass a larger limit (e.g. 10000) for a deliberate full inventory."
            }),
            json!({
                "name": "verbose",
                "in": "query",
                "required": false,
                "schema": {"type": "boolean"},
                "description": "Full list rows: 200-char title, created_at, family metadata (consolidated/outcome/source), full-precision updated_at. Default false = slim rows (id, title <=80, status, updated_at minute-precision UTC). Full content is GET-by-id only in both shapes."
            }),
        ];

        // Add enum values to status param if available
        if let Some(values) = config.status_values() {
            list_params[0]["schema"]["enum"] =
                Value::Array(values.iter().map(|v| Value::String(v.clone())).collect());
        }

        paths[&base_path] = json!({
            "get": {
                "operationId": format!("list_{}", slug_plural),
                "tags": ["entities"],
                "parameters": list_params,
                "responses": {
                    "200": {
                        "description": format!("List of {}s", config.slug),
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": format!("#/components/schemas/{}ListResponse", entity_name)
                                }
                            }
                        }
                    }
                }
            },
            "post": {
                "operationId": format!("create_{}", config.slug),
                "tags": ["entities"],
                "requestBody": {
                    "content": {
                        "application/json": {
                            "schema": {
                                "$ref": format!("#/components/schemas/Create{}Request", entity_name)
                            }
                        }
                    }
                },
                "responses": {
                    "201": {
                        "description": format!("{} created", entity_name),
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": format!("#/components/schemas/{}", entity_name)
                                }
                            }
                        }
                    }
                }
            }
        });

        // GET /{base}/{id} — get
        paths[&format!("{}/{{id}}", base_path)] = json!({
            "get": {
                "operationId": format!("get_{}", config.slug),
                "tags": ["entities"],
                "parameters": [
                    {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}
                ],
                "responses": {
                    "200": {
                        "description": format!("{} details", entity_name),
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": format!("#/components/schemas/{}", entity_name)
                                }
                            }
                        }
                    }
                }
            }
        });

        // PATCH /{base}/{id}/content — agent identity charters only (v0.3.0):
        // the governed identity-content update surface (tocpi calls this for
        // in-place re-chartering). Mirrors the tocpi content contract.
        if config.slug == "agent-identity" {
            paths[&format!("{}/{{id}}/content", base_path)] = json!({
                "patch": {
                    "operationId": format!("update_{}_content", config.slug),
                    "tags": ["entities"],
                    "parameters": [
                        {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                        {"name": "X-Instance-Id", "in": "header", "required": true, "schema": {"type": "string"}, "description": "Agent asset ID — identity content is instance-scoped (agent nested DB)"}
                    ],
                    "requestBody": {
                        "content": {
                            "application/json": {
                                "schema": {
                                    "type": "object",
                                    "properties": {
                                        "content": {"type": "string", "description": "New identity content (empty string = explicit wipe, logged)"}
                                    },
                                    "required": ["content"]
                                }
                            }
                        }
                    },
                    "responses": {
                        "200": {
                            "description": "Content updated in place (entity + old→new sha256 echo)",
                            "content": {
                                "application/json": {
                                    "schema": {
                                        "type": "object",
                                        "properties": {
                                            "content_hash": {
                                                "type": "object",
                                                "properties": {
                                                    "old": {"type": ["string", "null"]},
                                                    "new": {"type": "string"}
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        },
                        "400": {"description": "Missing X-Instance-Id or content field"},
                        "403": {"description": "Cedar deny — principal outside the stored admin_group"},
                        "404": {"description": "Identity not found (never create-on-miss)"}
                    }
                }
            });
        }

        // PATCH /{base}/{id}/status — the body schema carries the accepted
        // vocabulary when the entity declares one, so callers learn the
        // enum from the served spec itself (issues 80c2544e + 6abfa0b5).
        let mut status_body_schema = json!({
            "type": "object",
            "properties": {"status": {"type": "string"}},
            "required": ["status"]
        });
        if let Some(values) = config.status_values() {
            status_body_schema["properties"]["status"]["enum"] =
                Value::Array(values.iter().map(|v| Value::String(v.clone())).collect());
        }

        paths[&format!("{}/{{id}}/status", base_path)] = json!({
            "patch": {
                "operationId": format!("update_{}_status", config.slug),
                "tags": ["entities"],
                "parameters": [
                    {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}
                ],
                "requestBody": {
                    "content": {
                        "application/json": {
                            "schema": status_body_schema
                        }
                    }
                },
                "responses": {
                    "200": {
                        "description": "Status updated",
                        "content": {
                            "application/json": {
                                "schema": {
                                    "$ref": format!("#/components/schemas/{}", entity_name)
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // Ensure every schema object in the spec has a "required" field.
    // utoipa 5.x's OpenApi deserializer requires "required" on all Object schemas,
    // but the base spec and inline path schemas may omit it when empty.
    ensure_required_fields(&mut spec);

    // Deserialize back into OpenApi
    serde_json::from_value(spec).unwrap_or_else(|e| {
        tracing::error!("Failed to rebuild OpenAPI spec: {}", e);
        crate::models::ApiDoc::openapi()
    })
}

/// Map a [`FieldType`] to an OpenAPI schema type string.
fn field_type_to_openapi(ft: &FieldType) -> &'static str {
    match ft {
        FieldType::Text | FieldType::Markdown => "string",
        FieldType::Date => "string",
        FieldType::Enum => "string",
        FieldType::TagArray => "array",
        FieldType::Relation => "string",
        FieldType::Metadata => "object",
    }
}

/// Generate a JSON Schema object for the entity response.
fn generate_entity_schema_json(config: &EntityConfig) -> Value {
    let mut props = Map::new();

    // Always include id
    props.insert("id".to_string(), json!({"type": "string"}));

    for (field_name, field_config) in &config.fields {
        let type_str = field_type_to_openapi(&field_config.field_type);

        let mut field_schema = json!({"type": type_str});

        if let Some(values) = &field_config.values {
            field_schema["enum"] =
                Value::Array(values.iter().map(|v| Value::String(v.clone())).collect());
        }

        if let Some(label) = &field_config.label {
            field_schema["description"] = Value::String(label.clone());
        }

        props.insert(field_name.clone(), field_schema);
    }

    // Ensure created_at/updated_at exist
    if !props.contains_key("created_at") {
        props.insert(
            "created_at".to_string(),
            json!({"type": "string", "format": "date-time"}),
        );
    }
    if !props.contains_key("updated_at") {
        props.insert(
            "updated_at".to_string(),
            json!({"type": "string", "format": "date-time"}),
        );
    }

    let mut required: Vec<String> = config
        .fields
        .iter()
        .filter(|(_, fc)| fc.required)
        .map(|(name, _)| name.clone())
        .collect();
    required.push("id".to_string());

    json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

/// Generate the create request schema for an entity.
fn generate_create_schema_json(config: &EntityConfig) -> Value {
    let mut props = Map::new();
    let mut required: Vec<String> = Vec::new();

    for (field_name, field_config) in &config.fields {
        // Skip computed/system fields
        if matches!(
            field_config.map.as_deref(),
            Some("created_at") | Some("updated_at")
        ) {
            continue;
        }

        let type_str = field_type_to_openapi(&field_config.field_type);

        let mut field_schema = json!({"type": type_str});

        if let Some(values) = &field_config.values {
            field_schema["enum"] =
                Value::Array(values.iter().map(|v| Value::String(v.clone())).collect());
        }

        props.insert(field_name.clone(), field_schema);

        if field_config.required {
            required.push(field_name.clone());
        }
    }

    json!({
        "type": "object",
        "properties": props,
        "required": required,
    })
}

/// Recursively walk the spec JSON and ensure every object schema has a "required" field.
///
/// utoipa 5.x's `OpenApi` deserializer treats `required` as a non-optional field
/// on `Object` types. Schemas generated via `json!({})` or from older base specs
/// may omit it entirely, causing `missing field 'required'` deserialization errors.
/// This function patches all such schemas to include `"required": []` where missing.
fn ensure_required_fields(spec: &mut Value) {
    // Recursively walk the entire JSON tree. Any object that has "properties"
    // but no "required" gets patched. Also patches "parameters" in path operations.
    ensure_required_recursive(spec);
}

/// Recursive walker — patches any dict that looks like a schema (has "properties")
/// or a parameter (has "in") that's missing "required".
fn ensure_required_recursive(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Patch object schemas: has "properties" or type=="object" but no "required"
            let is_object_schema = map.get("type").and_then(|v| v.as_str()) == Some("object")
                || map.contains_key("properties");
            if is_object_schema && !map.contains_key("required") {
                map.insert("required".to_string(), json!([]));
            }

            // Patch parameters: has "in" (query/path/header/cookie) but no "required"
            if map.contains_key("in") && !map.contains_key("required") {
                map.insert("required".to_string(), json!(false));
            }

            // Recurse into all values
            for (_, v) in map.iter_mut() {
                ensure_required_recursive(v);
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                ensure_required_recursive(item);
            }
        }
        _ => {}
    }
}

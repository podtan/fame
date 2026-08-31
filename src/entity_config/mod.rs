//! Entity configuration system — TOML-driven entity definitions
//!
//! Each entity (e.g. "document") is defined in a TOML file under `entities/`.
//! This module provides the Rust types that those TOML files deserialize into,
//! plus a loader that scans a directory for `*.toml` config files.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

use crate::pdt::{PdtTag, PdtTagSummary};

/// Common read access to (category, value) for both tag shapes.
pub trait AsTagPair {
    fn category(&self) -> &str;
    fn value(&self) -> &str;
}

impl AsTagPair for PdtTag {
    fn category(&self) -> &str {
        &self.category
    }
    fn value(&self) -> &str {
        &self.value
    }
}

impl AsTagPair for PdtTagSummary {
    fn category(&self) -> &str {
        &self.category
    }
    fn value(&self) -> &str {
        &self.value
    }
}

/// The top-level entity file: `[entity] ...`
#[derive(Debug, Clone, Deserialize)]
pub struct EntityFile {
    pub entity: EntityConfig,
}

/// Main entity configuration parsed from the `[entity]` table.
#[derive(Debug, Clone, Deserialize)]
pub struct EntityConfig {
    /// Human-readable name (e.g. "Document")
    pub name: String,
    /// URL slug (e.g. "document" → `/api/v1/documents`)
    pub slug: String,
    /// Prefix prepended to asset titles in PDT (e.g. "Document: ")
    #[serde(default)]
    pub title_prefix: String,
    /// Handler type: "generic" or "custom"
    #[serde(default = "default_handler")]
    pub handler: String,
    /// Tags automatically applied to every asset of this entity type
    #[serde(default)]
    pub default_tags: HashMap<String, String>,
    /// Cedar authorization actions
    #[serde(default)]
    pub cedar: CedarActions,
    /// Field definitions keyed by field name
    #[serde(default)]
    pub fields: HashMap<String, FieldConfig>,
    /// View definitions keyed by view name (e.g. "list", "detail")
    #[serde(default)]
    pub views: HashMap<String, ViewConfig>,
}

fn default_handler() -> String {
    "generic".to_string()
}

/// Convert an entity slug to a Cedar-safe entity type name.
///
/// Cedar entity type names are identifiers: they may not contain spaces or
/// hyphens. Slugs are kebab-case ("semantic-memory"), display names contain
/// spaces ("Semantic Memory") — neither works as a Cedar resource type.
/// This maps "semantic-memory" → "SemanticMemory", matching the resource
/// types declared in policies/schema.cedarschema.
///
/// Non-alphanumeric characters terminate the identifier part; everything
/// after them is dropped, each surviving segment capitalized.
pub fn slug_to_cedar_type(slug: &str) -> String {
    slug.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut cs = s.chars();
            match cs.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Cedar authorization action names for this entity.
#[derive(Debug, Clone, Deserialize)]
pub struct CedarActions {
    #[serde(default)]
    pub action_create: Option<String>,
    #[serde(default)]
    pub action_read: Option<String>,
    #[serde(default)]
    pub action_edit: Option<String>,
    #[serde(default)]
    pub action_delete: Option<String>,
}

impl Default for CedarActions {
    fn default() -> Self {
        Self {
            action_create: None,
            action_read: None,
            action_edit: None,
            action_delete: None,
        }
    }
}

/// Supported field types in entity configs.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Markdown,
    Enum,
    Date,
    TagArray,
    Relation,
    Metadata,
}

impl Default for FieldType {
    fn default() -> Self {
        FieldType::Text
    }
}

/// Per-field configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct FieldConfig {
    /// Field data type
    #[serde(rename = "type", default)]
    pub field_type: FieldType,
    /// Whether the field is required on create
    #[serde(default)]
    pub required: bool,
    /// Human-readable label (defaults to field name)
    pub label: Option<String>,
    /// Mapping hint: "title_suffix", "content", "tag:X", "metadata:X",
    /// "created_at", "updated_at"
    pub map: Option<String>,
    /// Allowed values for enum fields
    pub values: Option<Vec<String>>,
    /// Default value for the field
    pub default: Option<String>,
    /// Whether to render as a badge in UI
    #[serde(default)]
    pub badge: bool,
    /// Which views this field appears in
    #[serde(default)]
    pub in_views: Vec<String>,
    /// Target entity for relation fields (e.g. "project", "any")
    pub target_entity: Option<String>,
    /// PDT relation type (e.g. "related_to", "contains")
    pub relation_type: Option<String>,
}

/// View layout configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ViewConfig {
    /// Layout style: "card", "detail", "form"
    pub layout: String,
    /// Field names to include in this view
    pub fields: Vec<String>,
    /// Optional sort configuration
    #[serde(default)]
    pub sort: Option<SortConfig>,
}

/// Sort configuration for views.
#[derive(Debug, Clone, Deserialize)]
pub struct SortConfig {
    /// Field name to sort by
    pub field: String,
    /// "asc" or "desc"
    #[serde(default = "default_sort_order")]
    pub order: String,
}

fn default_sort_order() -> String {
    "desc".to_string()
}

/// Load all entity configs from a directory.
///
/// Scans for `*.toml` files, parses each into an [`EntityConfig`], and returns
/// all successfully parsed configs. Files that fail to parse are logged and
/// skipped so one bad file doesn't prevent startup.
pub fn load_entity_configs(dir: &str) -> Vec<EntityConfig> {
    load_entity_configs_from_path(Path::new(dir))
}

/// Load all entity configs from a `Path`.
pub fn load_entity_configs_from_path(dir: &Path) -> Vec<EntityConfig> {
    let mut configs = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Cannot read entity config directory {:?}: {}", dir, e);
            return configs;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }

        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str::<EntityFile>(&content) {
                Ok(file) => {
                    tracing::info!("Loaded entity config: {} ({})", file.entity.name, filename);
                    configs.push(file.entity);
                }
                Err(e) => {
                    tracing::error!("Failed to parse entity config {}: {}", filename, e);
                }
            },
            Err(e) => {
                tracing::error!("Cannot read entity config {}: {}", filename, e);
            }
        }
    }

    configs
}

impl EntityConfig {
    /// Look up a field config by name.
    pub fn get_field(&self, name: &str) -> Option<&FieldConfig> {
        self.fields.get(name)
    }

    /// Get the tag value for `type` from default_tags.
    pub fn type_tag(&self) -> Option<&str> {
        self.default_tags.get("type").map(|s| s.as_str())
    }

    /// Check whether an asset's tag set matches this entity's default_tags.
    ///
    /// Entities that share a `type` tag (the three memory types all use
    /// `type=agent-memory`) are distinguished by their remaining default
    /// tags (e.g. `memory-type=episodic`). Every discriminator in
    /// default_tags must be present with the exact value; tags on the asset
    /// that are not part of default_tags are ignored.
    ///
    /// Generic over PdtTag / PdtTagSummary — full assets and compact
    /// search results carry the same (category, value) pairs.
    pub fn tags_match<T: AsTagPair>(&self, asset_tags: &[T]) -> bool {
        self.default_tags.iter().all(|(category, value)| {
            asset_tags
                .iter()
                .any(|t| t.category() == category && t.value() == value)
        })
    }

    /// Find all fields that should appear in a given view.
    pub fn fields_for_view(&self, view: &str) -> Vec<(&String, &FieldConfig)> {
        self.fields
            .iter()
            .filter(|(_, fc)| fc.in_views.iter().any(|v| v == view))
            .collect()
    }

    /// Find the default status value (from default_tags or the status field's `default`).
    pub fn default_status(&self) -> Option<&str> {
        self.default_tags
            .get("status")
            .map(|s| s.as_str())
            .or_else(|| self.fields.get("status").and_then(|f| f.default.as_deref()))
    }

    /// Get allowed status values if the status field is an enum.
    pub fn status_values(&self) -> Option<&[String]> {
        self.fields.get("status").and_then(|f| f.values.as_deref())
    }

    /// Find relation fields (field_type == Relation).
    pub fn relation_fields(&self) -> Vec<(&String, &FieldConfig)> {
        self.fields
            .iter()
            .filter(|(_, fc)| fc.field_type == FieldType::Relation)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_TOML: &str = r#"
[entity]
name = "Document"
slug = "document"
title_prefix = "Document: "
handler = "generic"

[entity.default_tags]
type = "document"
status = "draft"

[entity.cedar]
action_create = "CreateDocument"
action_read = "ViewDocument"
action_edit = "EditDocument"
action_delete = "DeleteDocument"

[entity.fields.title]
type = "text"
required = true
map = "title_suffix"
in_views = ["list", "detail"]

[entity.fields.content]
type = "markdown"
map = "content"
in_views = ["detail"]

[entity.fields.status]
type = "enum"
values = ["draft", "review", "approved", "published", "archived"]
default = "draft"
map = "tag:status"
badge = true
in_views = ["list", "detail"]

    [entity.fields.attached_to]
type = "relation"
target_entity = "any"
relation_type = "related_to"
required = true
in_views = ["list", "detail"]

[entity.fields.created_at]
type = "date"
map = "created_at"
in_views = ["list", "detail"]

[entity.fields.updated_at]
type = "date"
map = "updated_at"
in_views = ["detail"]

[entity.views.list]
layout = "card"
fields = ["title", "status", "updated_at"]

[entity.views.detail]
layout = "detail"
fields = ["title", "status", "content", "attached_to", "created_at", "updated_at"]
"#;

    #[test]
    fn test_parse_document_config() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).expect("Failed to parse sample TOML");
        let entity = &file.entity;

        assert_eq!(entity.name, "Document");
        assert_eq!(entity.slug, "document");
        assert_eq!(entity.title_prefix, "Document: ");
        assert_eq!(entity.handler, "generic");
    }

    #[test]
    fn test_default_tags() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).unwrap();
        let entity = &file.entity;

        assert_eq!(entity.type_tag(), Some("document"));
        assert_eq!(entity.default_status(), Some("draft"));
    }

    /// Regression for the cross-type memory leak: the three memory entities
    /// share type=agent-memory and are discriminated ONLY by memory-type.
    /// tags_match must require every default tag, and must work for both
    /// the compact (search result) and full (asset) tag shapes.
    #[test]
    fn test_tags_match_discriminates_shared_type() {
        let entity_toml = r#"
            [entity]
            name = "Episodic Memory"
            slug = "episodic-memory"
            title_prefix = "Episode: "
            handler = "generic"

            [entity.default_tags]
            type = "agent-memory"
            memory-type = "episodic"
        "#;
        let entity: EntityFile = toml::from_str(entity_toml).unwrap();
        let entity = &entity.entity;

        let summary = |c: &str, v: &str| PdtTagSummary {
            category: c.to_string(),
            value: v.to_string(),
        };
        let full = |c: &str, v: &str| PdtTag {
            id: "t".into(),
            category: c.to_string(),
            value: v.to_string(),
            added_by: String::new(),
            added_at: String::new(),
        };

        // An episodic memory: has both discriminators → matches.
        let episodic_summary = vec![
            summary("type", "agent-memory"),
            summary("memory-type", "episodic"),
        ];
        assert!(entity.tags_match(&episodic_summary));

        // A semantic memory: same type, different memory-type → must NOT match.
        let semantic_summary = vec![
            summary("type", "agent-memory"),
            summary("memory-type", "semantic"),
        ];
        assert!(!entity.tags_match(&semantic_summary));

        // Missing the discriminator entirely → must NOT match.
        let bare_summary = vec![summary("type", "agent-memory")];
        assert!(!entity.tags_match(&bare_summary));

        // Extra tags on the asset are ignored.
        let noisy_summary = vec![
            summary("type", "agent-memory"),
            summary("memory-type", "episodic"),
            summary("outcome", "success"),
        ];
        assert!(entity.tags_match(&noisy_summary));

        // Same guarantees for the full-asset shape (PdtTag).
        let episodic_full = vec![
            full("type", "agent-memory"),
            full("memory-type", "episodic"),
        ];
        assert!(entity.tags_match(&episodic_full));
        let semantic_full = vec![
            full("type", "agent-memory"),
            full("memory-type", "semantic"),
        ];
        assert!(!entity.tags_match(&semantic_full));
    }

    #[test]
    fn test_cedar_actions() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).unwrap();
        let entity = &file.entity;

        assert_eq!(
            entity.cedar.action_create.as_deref(),
            Some("CreateDocument")
        );
        assert_eq!(entity.cedar.action_read.as_deref(), Some("ViewDocument"));
        assert_eq!(entity.cedar.action_edit.as_deref(), Some("EditDocument"));
        assert_eq!(
            entity.cedar.action_delete.as_deref(),
            Some("DeleteDocument")
        );
    }

    #[test]
    fn test_field_configs() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).unwrap();
        let entity = &file.entity;

        // Title field
        let title = entity.get_field("title").unwrap();
        assert_eq!(title.field_type, FieldType::Text);
        assert!(title.required);
        assert_eq!(title.map.as_deref(), Some("title_suffix"));

        // Content field
        let content = entity.get_field("content").unwrap();
        assert_eq!(content.field_type, FieldType::Markdown);
        assert_eq!(content.map.as_deref(), Some("content"));

        // Status field
        let status = entity.get_field("status").unwrap();
        assert_eq!(status.field_type, FieldType::Enum);
        assert_eq!(
            status.values.as_deref(),
            Some(
                &[
                    "draft".to_string(),
                    "review".to_string(),
                    "approved".to_string(),
                    "published".to_string(),
                    "archived".to_string()
                ][..]
            )
        );
        assert_eq!(status.default.as_deref(), Some("draft"));
        assert!(status.badge);

        // Relation field
        let attached_to = entity.get_field("attached_to").unwrap();
        assert_eq!(attached_to.field_type, FieldType::Relation);
        assert_eq!(attached_to.target_entity.as_deref(), Some("any"));
        assert_eq!(attached_to.relation_type.as_deref(), Some("related_to"));

        // Date field
        let created_at = entity.get_field("created_at").unwrap();
        assert_eq!(created_at.field_type, FieldType::Date);
        assert_eq!(created_at.map.as_deref(), Some("created_at"));
    }

    #[test]
    fn test_views() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).unwrap();
        let entity = &file.entity;

        let list_view = entity.views.get("list").unwrap();
        assert_eq!(list_view.layout, "card");
        assert_eq!(list_view.fields, vec!["title", "status", "updated_at"]);

        let detail_view = entity.views.get("detail").unwrap();
        assert_eq!(detail_view.layout, "detail");
        assert_eq!(
            detail_view.fields,
            vec![
                "title",
                "status",
                "content",
                "attached_to",
                "created_at",
                "updated_at"
            ]
        );
    }

    #[test]
    fn test_fields_for_view() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).unwrap();
        let entity = &file.entity;

        let list_fields = entity.fields_for_view("list");
        let list_names: Vec<&str> = list_fields.iter().map(|(n, _)| n.as_str()).collect();
        assert!(list_names.contains(&"title"));
        assert!(list_names.contains(&"status"));
        assert!(!list_names.contains(&"content"));

        let detail_fields = entity.fields_for_view("detail");
        let detail_names: Vec<&str> = detail_fields.iter().map(|(n, _)| n.as_str()).collect();
        assert!(detail_names.contains(&"content"));
        assert!(detail_names.contains(&"attached_to"));
    }

    #[test]
    fn test_relation_fields() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).unwrap();
        let entity = &file.entity;

        let rels = entity.relation_fields();
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].0, "attached_to");
    }

    #[test]
    fn test_status_values() {
        let file: EntityFile = toml::from_str(SAMPLE_TOML).unwrap();
        let entity = &file.entity;

        let values = entity.status_values().unwrap();
        assert_eq!(values.len(), 5);
        assert!(values.contains(&"draft".to_string()));
        assert!(values.contains(&"archived".to_string()));
    }

    #[test]
    fn test_empty_cedar_defaults() {
        let toml_str = r#"
[entity]
name = "Test"
slug = "test"
"#;
        let file: EntityFile = toml::from_str(toml_str).unwrap();
        assert!(file.entity.cedar.action_create.is_none());
        assert!(file.entity.cedar.action_read.is_none());
    }

    #[test]
    fn test_handler_default() {
        let toml_str = r#"
[entity]
name = "Test"
slug = "test"
"#;
        let file: EntityFile = toml::from_str(toml_str).unwrap();
        assert_eq!(file.entity.handler, "generic");
    }

    #[test]
    fn test_slug_to_cedar_type() {
        assert_eq!(slug_to_cedar_type("semantic-memory"), "SemanticMemory");
        assert_eq!(slug_to_cedar_type("agent-identity"), "AgentIdentity");
        assert_eq!(slug_to_cedar_type("procedural-memory"), "ProceduralMemory");
        assert_eq!(slug_to_cedar_type("episodic-memory"), "EpisodicMemory");
        // Leading/trailing separators and accidental spaces are tolerated
        assert_eq!(slug_to_cedar_type(" semantic-memory "), "SemanticMemory");
        assert_eq!(slug_to_cedar_type("a"), "A");
        // Entity display names with spaces would be rejected by Cedar — this
        // documents why the slug, not `name`, feeds the resource UID
        assert_eq!(slug_to_cedar_type("Semantic Memory"), "SemanticMemory");
    }
}

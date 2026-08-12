//! PDT API Client

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct PdtClient {
    client: Client,
    base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdtTag {
    pub id: String,
    pub category: String,
    pub value: String,
    /// Added by user ID (returned by PDT, not used by NGHR)
    #[serde(default)]
    pub added_by: String,
    /// Timestamp when tag was added (returned by PDT, not used by NGHR)
    #[serde(default)]
    pub added_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthContext {
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub owner_groups: Vec<String>,
    #[serde(default)]
    pub confidentiality: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PdtAsset {
    #[serde(rename = "_id")]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub tags: Vec<PdtTag>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
    /// Creator user ID (returned by PDT, not used by NGHR)
    #[serde(default)]
    pub created_by: String,
    /// Last updater user ID (returned by PDT, not used by NGHR)
    #[serde(default)]
    pub updated_by: String,
    /// Soft-delete timestamp (returned by PDT, not used by NGHR)
    #[serde(default)]
    pub deleted_at: Option<String>,
    /// Cedar authorization context (visibility, owner_groups, confidentiality)
    #[serde(default)]
    pub auth_context: Option<AuthContext>,
}

/// Compact tag summary from search results (no id, added_by, added_at)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdtTagSummary {
    pub category: String,
    pub value: String,
}

/// Compact search result from PDT's /api/search (default, full_content=false)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdtSearchResult {
    #[serde(rename = "_id")]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub snippet: Option<String>,
    pub tags: Vec<PdtTagSummary>,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
struct CompactSearchResponse {
    data: Vec<PdtSearchResult>,
}

#[derive(Debug, Serialize)]
pub struct CreateAssetRequest {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<CreateTagRequest>>,
    /// Auth context to inherit from parent (visibility, owner_groups, confidentiality)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_context: Option<AuthContext>,
}

#[derive(Debug, Serialize)]
pub struct CreateTagRequest {
    pub category: String,
    pub value: String,
}

// --- Relation support ---

#[derive(Debug, Serialize)]
pub struct CreateRelationRequest {
    pub from_asset_id: String,
    pub to_asset_id: String,
    pub relation_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdtRelation {
    #[serde(rename = "_id")]
    pub id: String,
    pub from_asset_id: String,
    pub to_asset_id: String,
    pub relation_type: String,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    pub created_at: String,
}

impl PdtClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Get the base URL (for external use when needed)
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Get a reference to the HTTP client (for external use when needed)
    pub fn http_client(&self) -> &Client {
        &self.client
    }

    /// Build a request builder with the user's Bearer token and optional instance routing.
    fn req(&self, method: reqwest::Method, url: String, token: Option<&str>) -> reqwest::RequestBuilder {
        self.req_with_instance(method, url, token, None)
    }

    /// Build a request builder with token and instance ID for multi-tenant routing.
    fn req_with_instance(&self, method: reqwest::Method, url: String, token: Option<&str>, instance_id: Option<&str>) -> reqwest::RequestBuilder {
        let b = self.client.request(method, url);
        let b = if let Some(t) = token {
            b.bearer_auth(t)
        } else {
            b
        };
        let b = if let Some(id) = instance_id {
            b.header("X-Instance-Id", id)
        } else {
            b
        };
        b
    }

    /// Create an instance-scoped view of this client.
    /// Returns a wrapper that injects X-Instance-Id on every call.
    pub fn for_instance(&self, instance_id: Option<&str>) -> InstancePdtClient<'_> {
        InstancePdtClient {
            client: self,
            instance_id: instance_id.map(|s| s.to_string()),
        }
    }

    /// Search by tag — returns compact results (id, title, snippet, tags, updated_at).
    /// Use for list views where full content is not needed.
    pub async fn search_by_tag(&self, category: &str, value: &str, token: Option<&str>) -> Result<Vec<PdtSearchResult>> {
        self.search_by_tag_instance(category, value, token, None).await
    }

    /// Search by tag with instance routing.
    pub async fn search_by_tag_instance(&self, category: &str, value: &str, token: Option<&str>, instance_id: Option<&str>) -> Result<Vec<PdtSearchResult>> {
        let url = format!("{}/api/search?tag={}:{}&limit=100", self.base_url, category, value);
        let resp = self.req_with_instance(reqwest::Method::GET, url, token, instance_id).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PDT search failed ({}): {}", status, body);
        }
        let result = resp.json::<CompactSearchResponse>().await?;
        Ok(result.data)
    }

    /// Full-text search for skill assets — wraps PDT's /api/search with type:skill tag filter.
    /// Returns compact results suitable for skill discovery and trigger matching.
    pub async fn search_skills(&self, query: &str, limit: i64, token: Option<&str>) -> Result<Vec<PdtSearchResult>> {
        let encoded_query: String = query
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c.to_string() } else { format!("%{:02X}", c as u8) })
            .collect();
        let url = format!(
            "{}/api/search?q={}&tag=type:skill&limit={}",
            self.base_url,
            encoded_query,
            limit
        );
        let resp = self.req(reqwest::Method::GET, url, token).send().await?;
        let result = resp.json::<CompactSearchResponse>().await?;
        Ok(result.data)
    }

    pub async fn get_asset(&self, id: &str, token: Option<&str>) -> Result<PdtAsset> {
        self.get_asset_instance(id, token, None).await
    }

    /// Get asset with instance routing.
    pub async fn get_asset_instance(&self, id: &str, token: Option<&str>, instance_id: Option<&str>) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.base_url, id);
        let resp = self.req_with_instance(reqwest::Method::GET, url, token, instance_id).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PDT get_asset failed ({}): {}", status, body);
        }
        Ok(resp.json::<PdtAsset>().await?)
    }

    pub async fn create_asset(&self, req: CreateAssetRequest, token: Option<&str>) -> Result<PdtAsset> {
        self.create_asset_instance(req, token, None).await
    }

    /// Create asset with instance routing.
    pub async fn create_asset_instance(&self, req: CreateAssetRequest, token: Option<&str>, instance_id: Option<&str>) -> Result<PdtAsset> {
        let url = format!("{}/api/assets", self.base_url);
        let resp = self.req_with_instance(reqwest::Method::POST, url, token, instance_id).json(&req).send().await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_content(&self, id: &str, content: &str, token: Option<&str>) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.base_url, id);
        let resp = self.req(reqwest::Method::PUT, url, token).json(&serde_json::json!({ "content": content })).send().await?;
        parse_pdt_response(resp).await
    }

    /// Create a relation between two assets
    pub async fn create_relation(&self, req: CreateRelationRequest, token: Option<&str>) -> Result<PdtRelation> {
        let url = format!("{}/api/relations", self.base_url);
        let resp = self.req(reqwest::Method::POST, url, token).json(&req).send().await?;
        Ok(resp.json::<PdtRelation>().await?)
    }

    /// Get all relations for an asset.
    /// Note: PDT returns ALL relations regardless of direction param, so callers
    /// must filter client-side by checking from_asset_id / to_asset_id.
    pub async fn get_relations(&self, asset_id: &str, _direction: Option<&str>, token: Option<&str>) -> Result<Vec<PdtRelation>> {
        let url = format!("{}/api/assets/{}/relations", self.base_url, asset_id);
        let resp = self.req(reqwest::Method::GET, url, token).send().await?;
        Ok(resp.json().await?)
    }

    /// Delete a relation by its ID
    pub async fn delete_relation(&self, relation_id: &str, token: Option<&str>) -> Result<()> {
        let url = format!("{}/api/relations/{}", self.base_url, relation_id);
        self.req(reqwest::Method::DELETE, url, token).send().await?;
        Ok(())
    }

    /// Update asset title
    pub async fn update_asset_title(&self, id: &str, title: &str, token: Option<&str>) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.base_url, id);
        let resp = self.req(reqwest::Method::PUT, url, token).json(&serde_json::json!({ "title": title })).send().await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_tag(&self, id: &str, category: &str, value: &str, token: Option<&str>) -> Result<PdtAsset> {
        let asset = self.get_asset(id, token).await?;
        
        let existing_tag_id = asset.tags.iter()
            .find(|t| t.category == category)
            .map(|t| t.id.as_str());
        
        if let Some(tag_id) = existing_tag_id {
            let url = format!("{}/api/assets/{}/tags/{}", self.base_url, id, tag_id);
            self.req(reqwest::Method::DELETE, url, token).send().await?;
        }
        
        let url = format!("{}/api/assets/{}/tags", self.base_url, id);
        self.req(reqwest::Method::POST, url, token).json(&CreateTagRequest {
            category: category.to_string(),
            value: value.to_string(),
        }).send().await?;
        
        self.get_asset(id, token).await
    }

    /// Get the auth_context of an asset (for inheritance at creation).
    pub async fn get_auth_context(&self, id: &str, token: Option<&str>) -> Result<Option<AuthContext>> {
        let asset = self.get_asset(id, token).await?;
        Ok(asset.auth_context)
    }

    /// Update the auth_context of an asset, optionally cascading to descendants.
    pub async fn update_auth_context(
        &self,
        id: &str,
        visibility: Option<&str>,
        owner_groups: Option<&[String]>,
        confidentiality: Option<&str>,
        cascade: bool,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}/auth-context", self.base_url, id);
        let mut body = serde_json::Map::new();
        if let Some(v) = visibility {
            body.insert("visibility".to_string(), serde_json::Value::String(v.to_string()));
        }
        if let Some(g) = owner_groups {
            body.insert("owner_groups".to_string(), serde_json::to_value(g)?);
        }
        if let Some(c) = confidentiality {
            body.insert("confidentiality".to_string(), serde_json::Value::String(c.to_string()));
        }
        body.insert("cascade".to_string(), serde_json::Value::Bool(cascade));

        let resp = self.req(reqwest::Method::PUT, url, token)
            .json(&serde_json::Value::Object(body))
            .send().await?;
        parse_pdt_response(resp).await
    }
}

/// Wrapper for PDT API responses that may be nested under a "data" key
#[derive(Debug, Deserialize, Default)]
struct PdtResponse<T: Default> {
    #[serde(default)]
    data: Option<T>,
}

async fn parse_pdt_response<T: serde::de::DeserializeOwned + Default>(resp: reqwest::Response) -> Result<T> {
    // Try direct deserialization first (PDT may return the object directly)
    let bytes = resp.bytes().await?;
    
    // First try: direct deserialization
    if let Ok(val) = serde_json::from_slice::<T>(&bytes) {
        return Ok(val);
    }
    
    // Second try: wrapped in { "data": ... }
    if let Ok(wrapped) = serde_json::from_slice::<PdtResponse<T>>(&bytes) {
        if let Some(data) = wrapped.data {
            return Ok(data);
        }
    }
    
    // If both fail, try direct one more time with better error
    serde_json::from_slice::<T>(&bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse PDT response: {} | body: {}", e, String::from_utf8_lossy(&bytes)))
}

impl PdtSearchResult {
    pub fn get_tag(&self, category: &str) -> Option<&str> {
        self.tags.iter()
            .find(|t| t.category == category)
            .map(|t| t.value.as_str())
    }
}

impl PdtAsset {
    pub fn get_tag(&self, category: &str) -> Option<&str> {
        self.tags.iter()
            .find(|t| t.category == category)
            .map(|t| t.value.as_str())
    }
}

/// Instance-scoped PDT client wrapper.
///
/// Carries an optional instance_id and injects X-Instance-Id header on every call.
/// Created via `pdt_client.for_instance(Some(instance_id))`.
pub struct InstancePdtClient<'a> {
    client: &'a PdtClient,
    instance_id: Option<String>,
}

impl<'a> InstancePdtClient<'a> {
    pub async fn search_by_tag(&self, category: &str, value: &str, token: Option<&str>) -> Result<Vec<PdtSearchResult>> {
        self.client.search_by_tag_instance(category, value, token, self.instance_id.as_deref()).await
    }

    pub async fn search_skills(&self, query: &str, limit: i64, token: Option<&str>) -> Result<Vec<PdtSearchResult>> {
        let url = format!(
            "{}/api/search?q={}&tag=type:skill&limit={}",
            self.client.base_url,
            query.replace(' ', "+"),
            limit
        );
        let resp = self.client.req_with_instance(reqwest::Method::GET, url, token, self.instance_id.as_deref()).send().await?;
        let result = resp.json::<CompactSearchResponse>().await?;
        Ok(result.data)
    }

    pub async fn get_asset(&self, id: &str, token: Option<&str>) -> Result<PdtAsset> {
        self.client.get_asset_instance(id, token, self.instance_id.as_deref()).await
    }

    pub async fn create_asset(&self, req: CreateAssetRequest, token: Option<&str>) -> Result<PdtAsset> {
        self.client.create_asset_instance(req, token, self.instance_id.as_deref()).await
    }

    pub async fn update_asset_content(&self, id: &str, content: &str, token: Option<&str>) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.client.base_url, id);
        let resp = self.client.req_with_instance(reqwest::Method::PUT, url, token, self.instance_id.as_deref())
            .json(&serde_json::json!({ "content": content }))
            .send().await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_title(&self, id: &str, title: &str, token: Option<&str>) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.client.base_url, id);
        let resp = self.client.req_with_instance(reqwest::Method::PUT, url, token, self.instance_id.as_deref())
            .json(&serde_json::json!({ "title": title }))
            .send().await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_tag(&self, id: &str, category: &str, value: &str, token: Option<&str>) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}/tags", self.client.base_url, id);
        self.client.req_with_instance(reqwest::Method::POST, url, token, self.instance_id.as_deref())
            .json(&CreateTagRequest {
                category: category.to_string(),
                value: value.to_string(),
            })
            .send().await?;
        // Return updated asset
        let url2 = format!("{}/api/assets/{}", self.client.base_url, id);
        let resp = self.client.req_with_instance(reqwest::Method::GET, url2, token, self.instance_id.as_deref()).send().await?;
        Ok(resp.json::<PdtAsset>().await?)
    }

    pub async fn create_relation(&self, req: CreateRelationRequest, token: Option<&str>) -> Result<PdtRelation> {
        let url = format!("{}/api/relations", self.client.base_url);
        let resp = self.client.req_with_instance(reqwest::Method::POST, url, token, self.instance_id.as_deref())
            .json(&req)
            .send().await?;
        Ok(resp.json::<PdtRelation>().await?)
    }

    pub async fn get_relations(&self, asset_id: &str, _direction: Option<&str>, token: Option<&str>) -> Result<Vec<PdtRelation>> {
        let url = format!("{}/api/assets/{}/relations", self.client.base_url, asset_id);
        let resp = self.client.req_with_instance(reqwest::Method::GET, url, token, self.instance_id.as_deref()).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PDT get_relations failed ({}): {}", status, body);
        }
        Ok(resp.json().await?)
    }

    pub async fn delete_relation(&self, relation_id: &str, token: Option<&str>) -> Result<()> {
        let url = format!("{}/api/relations/{}", self.client.base_url, relation_id);
        self.client.req_with_instance(reqwest::Method::DELETE, url, token, self.instance_id.as_deref()).send().await?;
        Ok(())
    }

    pub async fn get_auth_context(&self, id: &str, token: Option<&str>) -> Result<Option<AuthContext>> {
        let url = format!("{}/api/assets/{}", self.client.base_url, id);
        let resp = self.client.req_with_instance(reqwest::Method::GET, url, token, self.instance_id.as_deref()).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PDT get_asset failed ({}): {}", status, body);
        }
        let asset: PdtAsset = resp.json().await?;
        Ok(asset.auth_context)
    }

    pub async fn update_auth_context(
        &self,
        id: &str,
        visibility: Option<&str>,
        owner_groups: Option<&[String]>,
        confidentiality: Option<&str>,
        cascade: bool,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}/auth-context", self.client.base_url, id);
        let mut body = serde_json::Map::new();
        if let Some(v) = visibility {
            body.insert("visibility".to_string(), serde_json::Value::String(v.to_string()));
        }
        if let Some(g) = owner_groups {
            body.insert("owner_groups".to_string(), serde_json::to_value(g)?);
        }
        if let Some(c) = confidentiality {
            body.insert("confidentiality".to_string(), serde_json::Value::String(c.to_string()));
        }
        body.insert("cascade".to_string(), serde_json::Value::Bool(cascade));

        let resp = self.client.req_with_instance(reqwest::Method::PUT, url, token, self.instance_id.as_deref())
            .json(&serde_json::Value::Object(body))
            .send().await?;
        parse_pdt_response(resp).await
    }

    pub fn base_url(&self) -> &str {
        &self.client.base_url
    }

    pub fn http_client(&self) -> &Client {
        &self.client.client
    }
}

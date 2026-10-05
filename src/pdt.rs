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

/// Deserialize `null` as `Default::default()`.
///
/// `#[serde(default)]` alone covers a MISSING field but still rejects an
/// EXPLICIT `null`. PDT is the authority on asset shape, but storage tiers
/// (sqlite vs mongo migration paths) and older writers can emit nulls for
/// fields that are structurally optional (timestamps on untouched rows,
/// metadata on bare creates, tag ids from bulk imports). Fame must be able
/// to read ANY asset PDT can serve — the strict-decode tier once 404'd a
/// live, correctly-banded asset because its payload carried a null
/// (issue 0473c444: GET 404 + status-update 500 with "error decoding
/// response body" while the sqlite row was verifiably intact).
///
/// Pair with `#[serde(default)]`: default covers absence, this covers null.
fn null_as_default<'de, T, D>(d: D) -> Result<T, D::Error>
where
    T: serde::Deserialize<'de> + Default,
    D: serde::Deserializer<'de>,
{
    let v: Option<T> = Option::deserialize(d)?;
    Ok(v.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdtTag {
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub id: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub category: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub value: String,
    /// Added by user ID (returned by PDT, not used by NGHR)
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub added_by: String,
    /// Timestamp when tag was added (returned by PDT, not used by NGHR)
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub added_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthContext {
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub visibility: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub owner_groups: Vec<String>,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub confidentiality: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PdtAsset {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub title: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub content: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub tags: Vec<PdtTag>,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub metadata: HashMap<String, serde_json::Value>,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub created_at: String,
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub updated_at: String,
    /// Creator user ID (returned by PDT, not used by NGHR)
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub created_by: String,
    /// Last updater user ID (returned by PDT, not used by NGHR)
    #[serde(default)]
    #[serde(deserialize_with = "null_as_default")]
    pub updated_by: String,
    /// Soft-delete timestamp (returned by PDT, not used by NGHR)
    #[serde(default)]
    pub deleted_at: Option<String>,
    /// Cedar authorization context (visibility, owner_groups, confidentiality)
    #[serde(default)]
    pub auth_context: Option<AuthContext>,
}

/// Outcome of an asset fetch, distinguishing TRUE absence (PDT answered
/// 404) from transport/decode failures. Callers must never report a decode
/// failure as "not found" — that masks live data behind a bug (issue
/// 0473c444: a live, correctly-banded procedural memory read as 404 for
/// days because its payload tripped the strict decode, and the get handler
/// mapped every fetch error to NOT_FOUND).
#[derive(Debug)]
pub enum AssetFetch {
    Found(PdtAsset),
    /// PDT answered 404 for this id — genuinely absent (or wrong instance).
    NotFound,
    /// Transport or decode failure — fame-side or PDT-side fault, NOT absence.
    Failed(anyhow::Error),
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
    fn req(
        &self,
        method: reqwest::Method,
        url: String,
        token: Option<&str>,
    ) -> reqwest::RequestBuilder {
        self.req_with_instance(method, url, token, None)
    }

    /// Build a request builder with token and instance ID for multi-tenant routing.
    fn req_with_instance(
        &self,
        method: reqwest::Method,
        url: String,
        token: Option<&str>,
        instance_id: Option<&str>,
    ) -> reqwest::RequestBuilder {
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
    pub async fn search_by_tag(
        &self,
        category: &str,
        value: &str,
        token: Option<&str>,
    ) -> Result<Vec<PdtSearchResult>> {
        self.search_by_tag_instance(category, value, token, None)
            .await
    }

    /// Search by tag with instance routing.
    ///
    /// Page size is 1000 (raised from 100, issue b3209637): the old cap made
    /// list endpoints a bounded window — records past the cap silently left
    /// every list while staying GET-able by id, and `total` (reported as
    /// entities.len() downstream) under-counted. 1000 is headroom, not a
    /// fix: the proper remedy is true-total semantics + cursor pagination.
    pub async fn search_by_tag_instance(
        &self,
        category: &str,
        value: &str,
        token: Option<&str>,
        instance_id: Option<&str>,
    ) -> Result<Vec<PdtSearchResult>> {
        let url = format!(
            "{}/api/search?tag={}:{}&limit=1000",
            self.base_url, category, value
        );
        let resp = self
            .req_with_instance(reqwest::Method::GET, url, token, instance_id)
            .send()
            .await?;
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
    pub async fn search_skills(
        &self,
        query: &str,
        limit: i64,
        token: Option<&str>,
    ) -> Result<Vec<PdtSearchResult>> {
        let encoded_query: String = query
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c.to_string()
                } else {
                    format!("%{:02X}", c as u8)
                }
            })
            .collect();
        let url = format!(
            "{}/api/search?q={}&tag=type:skill&limit={}",
            self.base_url, encoded_query, limit
        );
        let resp = self.req(reqwest::Method::GET, url, token).send().await?;
        let result = resp.json::<CompactSearchResponse>().await?;
        Ok(result.data)
    }

    pub async fn get_asset(&self, id: &str, token: Option<&str>) -> Result<PdtAsset> {
        self.get_asset_instance(id, token, None).await
    }

    /// Fetch an asset with absence/failure discrimination and loud failure
    /// logging. `get_asset` keeps the anyhow signature for existing callers;
    /// new handler paths that must distinguish "absent" from "broken" use
    /// this (issue 0473c444).
    pub async fn fetch_asset(
        &self,
        id: &str,
        token: Option<&str>,
        instance_id: Option<&str>,
    ) -> AssetFetch {
        match self.get_asset_instance(id, token, instance_id).await {
            Ok(asset) => AssetFetch::Found(asset),
            Err(e) => {
                let msg = e.to_string();
                if msg.starts_with("PDT asset not found") {
                    AssetFetch::NotFound
                } else {
                    // Fail LOUD at the choke point: a 500 with no journal
                    // line is its own bug (issue 0473c444 secondary finding).
                    tracing::error!(asset_id = %id, "PDT asset fetch failed: {msg}");
                    AssetFetch::Failed(e)
                }
            }
        }
    }

    /// Get asset with instance routing.
    pub async fn get_asset_instance(
        &self,
        id: &str,
        token: Option<&str>,
        instance_id: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.base_url, id);
        let resp = self
            .req_with_instance(reqwest::Method::GET, url, token, instance_id)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::NOT_FOUND {
                anyhow::bail!("PDT asset not found (404): {id}");
            }
            anyhow::bail!("PDT get_asset failed ({}): {}", status, body);
        }
        Ok(resp.json::<PdtAsset>().await?)
    }

    pub async fn create_asset(
        &self,
        req: CreateAssetRequest,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        self.create_asset_instance(req, token, None).await
    }

    /// Create asset with instance routing.
    pub async fn create_asset_instance(
        &self,
        req: CreateAssetRequest,
        token: Option<&str>,
        instance_id: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets", self.base_url);
        let resp = self
            .req_with_instance(reqwest::Method::POST, url, token, instance_id)
            .json(&req)
            .send()
            .await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_content(
        &self,
        id: &str,
        content: &str,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.base_url, id);
        let resp = self
            .req(reqwest::Method::PUT, url, token)
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await?;
        parse_pdt_response(resp).await
    }

    /// Create a relation between two assets
    pub async fn create_relation(
        &self,
        req: CreateRelationRequest,
        token: Option<&str>,
    ) -> Result<PdtRelation> {
        let url = format!("{}/api/relations", self.base_url);
        let resp = self
            .req(reqwest::Method::POST, url, token)
            .json(&req)
            .send()
            .await?;
        Ok(resp.json::<PdtRelation>().await?)
    }

    /// Get all relations for an asset.
    /// Note: PDT returns ALL relations regardless of direction param, so callers
    /// must filter client-side by checking from_asset_id / to_asset_id.
    pub async fn get_relations(
        &self,
        asset_id: &str,
        _direction: Option<&str>,
        token: Option<&str>,
    ) -> Result<Vec<PdtRelation>> {
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
    pub async fn update_asset_title(
        &self,
        id: &str,
        title: &str,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.base_url, id);
        let resp = self
            .req(reqwest::Method::PUT, url, token)
            .json(&serde_json::json!({ "title": title }))
            .send()
            .await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_tag(
        &self,
        id: &str,
        category: &str,
        value: &str,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let asset = self.get_asset(id, token).await?;

        let existing_tag_id = asset
            .tags
            .iter()
            .find(|t| t.category == category)
            .map(|t| t.id.as_str());

        if let Some(tag_id) = existing_tag_id {
            let url = format!("{}/api/assets/{}/tags/{}", self.base_url, id, tag_id);
            self.req(reqwest::Method::DELETE, url, token).send().await?;
        }

        let url = format!("{}/api/assets/{}/tags", self.base_url, id);
        self.req(reqwest::Method::POST, url, token)
            .json(&CreateTagRequest {
                category: category.to_string(),
                value: value.to_string(),
            })
            .send()
            .await?;

        self.get_asset(id, token).await
    }

    /// Get the auth_context of an asset (for inheritance at creation).
    pub async fn get_auth_context(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<Option<AuthContext>> {
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
            body.insert(
                "visibility".to_string(),
                serde_json::Value::String(v.to_string()),
            );
        }
        if let Some(g) = owner_groups {
            body.insert("owner_groups".to_string(), serde_json::to_value(g)?);
        }
        if let Some(c) = confidentiality {
            body.insert(
                "confidentiality".to_string(),
                serde_json::Value::String(c.to_string()),
            );
        }
        body.insert("cascade".to_string(), serde_json::Value::Bool(cascade));

        let resp = self
            .req(reqwest::Method::PUT, url, token)
            .json(&serde_json::Value::Object(body))
            .send()
            .await?;
        parse_pdt_response(resp).await
    }
}

/// Wrapper for PDT API responses that may be nested under a "data" key
#[derive(Debug, Deserialize, Default)]
struct PdtResponse<T: Default> {
    #[serde(default)]
    data: Option<T>,
}

async fn parse_pdt_response<T: serde::de::DeserializeOwned + Default>(
    resp: reqwest::Response,
) -> Result<T> {
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
    serde_json::from_slice::<T>(&bytes).map_err(|e| {
        anyhow::anyhow!(
            "Failed to parse PDT response: {} | body: {}",
            e,
            String::from_utf8_lossy(&bytes)
        )
    })
}

impl PdtSearchResult {
    pub fn get_tag(&self, category: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|t| t.category == category)
            .map(|t| t.value.as_str())
    }
}

impl PdtAsset {
    pub fn get_tag(&self, category: &str) -> Option<&str> {
        self.tags
            .iter()
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
    pub async fn search_by_tag(
        &self,
        category: &str,
        value: &str,
        token: Option<&str>,
    ) -> Result<Vec<PdtSearchResult>> {
        self.client
            .search_by_tag_instance(category, value, token, self.instance_id.as_deref())
            .await
    }

    pub async fn search_skills(
        &self,
        query: &str,
        limit: i64,
        token: Option<&str>,
    ) -> Result<Vec<PdtSearchResult>> {
        let url = format!(
            "{}/api/search?q={}&tag=type:skill&limit={}",
            self.client.base_url,
            query.replace(' ', "+"),
            limit
        );
        let resp = self
            .client
            .req_with_instance(
                reqwest::Method::GET,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .send()
            .await?;
        let result = resp.json::<CompactSearchResponse>().await?;
        Ok(result.data)
    }

    pub async fn get_asset(&self, id: &str, token: Option<&str>) -> Result<PdtAsset> {
        self.client
            .get_asset_instance(id, token, self.instance_id.as_deref())
            .await
    }

    /// Absence/failure-discriminating fetch (see [`PdtClient::fetch_asset`]).
    pub async fn fetch_asset(&self, id: &str, token: Option<&str>) -> AssetFetch {
        self.client
            .fetch_asset(id, token, self.instance_id.as_deref())
            .await
    }

    pub async fn create_asset(
        &self,
        req: CreateAssetRequest,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        self.client
            .create_asset_instance(req, token, self.instance_id.as_deref())
            .await
    }

    pub async fn update_asset_content(
        &self,
        id: &str,
        content: &str,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.client.base_url, id);
        let resp = self
            .client
            .req_with_instance(
                reqwest::Method::PUT,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_title(
        &self,
        id: &str,
        title: &str,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}", self.client.base_url, id);
        let resp = self
            .client
            .req_with_instance(
                reqwest::Method::PUT,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .json(&serde_json::json!({ "title": title }))
            .send()
            .await?;
        parse_pdt_response(resp).await
    }

    pub async fn update_asset_tag(
        &self,
        id: &str,
        category: &str,
        value: &str,
        token: Option<&str>,
    ) -> Result<PdtAsset> {
        let url = format!("{}/api/assets/{}/tags", self.client.base_url, id);
        self.client
            .req_with_instance(
                reqwest::Method::POST,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .json(&CreateTagRequest {
                category: category.to_string(),
                value: value.to_string(),
            })
            .send()
            .await?;
        // Return updated asset — with status discrimination: the final GET
        // must NEVER be decoded unchecked. A PDT 404 here (absent-or-deleted
        // in the routed store) decoded as PdtAsset surfaces as reqwest's
        // opaque "error decoding response body", which misdirected the
        // 0473c444 investigation for a full round. (0.3.12)
        match self.fetch_asset(id, token).await {
            AssetFetch::Found(asset) => Ok(asset),
            AssetFetch::NotFound => anyhow::bail!("PDT asset not found (404): {id}"),
            AssetFetch::Failed(e) => Err(e),
        }
    }

    pub async fn create_relation(
        &self,
        req: CreateRelationRequest,
        token: Option<&str>,
    ) -> Result<PdtRelation> {
        let url = format!("{}/api/relations", self.client.base_url);
        let resp = self
            .client
            .req_with_instance(
                reqwest::Method::POST,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .json(&req)
            .send()
            .await?;
        Ok(resp.json::<PdtRelation>().await?)
    }

    pub async fn get_relations(
        &self,
        asset_id: &str,
        _direction: Option<&str>,
        token: Option<&str>,
    ) -> Result<Vec<PdtRelation>> {
        let url = format!("{}/api/assets/{}/relations", self.client.base_url, asset_id);
        let resp = self
            .client
            .req_with_instance(
                reqwest::Method::GET,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("PDT get_relations failed ({}): {}", status, body);
        }
        Ok(resp.json().await?)
    }

    pub async fn delete_relation(&self, relation_id: &str, token: Option<&str>) -> Result<()> {
        let url = format!("{}/api/relations/{}", self.client.base_url, relation_id);
        self.client
            .req_with_instance(
                reqwest::Method::DELETE,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .send()
            .await?;
        Ok(())
    }

    pub async fn get_auth_context(
        &self,
        id: &str,
        token: Option<&str>,
    ) -> Result<Option<AuthContext>> {
        let url = format!("{}/api/assets/{}", self.client.base_url, id);
        let resp = self
            .client
            .req_with_instance(
                reqwest::Method::GET,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .send()
            .await?;
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
            body.insert(
                "visibility".to_string(),
                serde_json::Value::String(v.to_string()),
            );
        }
        if let Some(g) = owner_groups {
            body.insert("owner_groups".to_string(), serde_json::to_value(g)?);
        }
        if let Some(c) = confidentiality {
            body.insert(
                "confidentiality".to_string(),
                serde_json::Value::String(c.to_string()),
            );
        }
        body.insert("cascade".to_string(), serde_json::Value::Bool(cascade));

        let resp = self
            .client
            .req_with_instance(
                reqwest::Method::PUT,
                url,
                token,
                self.instance_id.as_deref(),
            )
            .json(&serde_json::Value::Object(body))
            .send()
            .await?;
        parse_pdt_response(resp).await
    }

    pub fn base_url(&self) -> &str {
        &self.client.base_url
    }

    pub fn http_client(&self) -> &Client {
        &self.client.client
    }
}

#[cfg(test)]
mod pdt_asset_decode_tolerance {
    //! Issue 0473c444: fame must decode ANY asset payload PDT can serve.
    //! Storage tiers (sqlite vs mongo) and older writers can emit nulls for
    /// structurally-optional fields; the strict decode once turned a live,
    /// correctly-banded asset into a 404. These pin the tolerant contract:
    /// null/missing optional fields decode to defaults; a payload without
    /// _id still fails (that is a genuinely malformed asset).
    use super::PdtAsset;

    fn base() -> String {
        r#"{"_id":"x","title":"t","content":"c","tags":[],"metadata":{},"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z","created_by":"u","updated_by":"u","auth_context":{"visibility":"team","owner_groups":["g"],"confidentiality":""}}"#.to_string()
    }

    #[test]
    fn tolerant_shapes_decode() {
        let cases: Vec<(&str, String)> = vec![
            ("baseline", base()),
            ("created_at missing", base().replace(r#","created_at":"2026-01-01T00:00:00Z""#, "")),
            ("created_at null", base().replace(r#""created_at":"2026-01-01T00:00:00Z""#, r#""created_at":null"#)),
            ("updated_at missing", base().replace(r#","updated_at":"2026-01-01T00:00:00Z""#, "")),
            ("updated_at null", base().replace(r#""updated_at":"2026-01-01T00:00:00Z""#, r#""updated_at":null"#)),
            ("metadata null", base().replace(r#""metadata":{}"#, r#""metadata":null"#)),
            ("metadata missing", base().replace(r#","metadata":{}"#, "")),
            ("tags null", base().replace(r#""tags":[]"#, r#""tags":null"#)),
            ("tag value null", base().replace(r#""tags":[]"#, r#""tags":[{"id":"t1","category":"status","value":null}]"#)),
            ("tag id missing", base().replace(r#""tags":[]"#, r#""tags":[{"category":"status","value":"draft"}]"#)),
            ("auth_context missing", base().replace(r#","auth_context":{"visibility":"team","owner_groups":["g"],"confidentiality":""}"#, "")),
            ("auth_context null", base().replace(r#","auth_context":{"visibility":"team","owner_groups":["g"],"confidentiality":""}"#, r#","auth_context":null"#)),
            ("auth owner_groups null", base().replace(r#""owner_groups":["g"]"#, r#""owner_groups":null"#)),
            ("auth visibility null", base().replace(r#""visibility":"team""#, r#""visibility":null"#)),
            ("title null", base().replace(r#""title":"t""#, r#""title":null"#)),
            ("unicode star emdash", base().replace("\"c\"", "\"\u{2605} \u{2014} dash\"")),
            ("auth_context extra field", base().replace(r#""confidentiality":""}"#, r#""confidentiality":"","unknown":123}"#)),
        ];
        for (name, payload) in &cases {
            let r: Result<PdtAsset, _> = serde_json::from_str(payload);
            assert!(r.is_ok(), "shape `{}` must decode: {:?}", name, r.err());
        }
    }

    #[test]
    fn genuinely_malformed_still_fails() {
        // Missing _id: a genuinely malformed asset — decode must refuse,
        // and the handler path surfaces it as a LOUD 500, never a 404.
        let payload = base().replace(r#""_id":"x","#, "");
        let r: Result<PdtAsset, _> = serde_json::from_str(&payload);
        assert!(r.is_err(), "missing _id must not decode");

        // auth_context of the WRONG TYPE (string where object belongs):
        // deliberately NOT tolerated. Swallowing it into None would silently
        // strip the asset's banding — the born-invisible class. A malformed
        // band fails loud (500 + journal line) instead.
        let payload = base().replace(
            r#"{"visibility":"team","owner_groups":["g"],"confidentiality":""}"#,
            "\"weird\"",
        );
        let r: Result<PdtAsset, _> = serde_json::from_str(&payload);
        assert!(r.is_err(), "wrong-typed auth_context must not decode");
    }
}

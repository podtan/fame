use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use pep::oidc::types::JwtClaims;
use serde_json::Value;

/// The raw Bearer token from the incoming request, stored in request extensions
/// by the auth middleware. Used to relay the user's token to downstream services
/// (e.g. PDT) so that `created_by` is stamped with the real user identity.
#[derive(Debug, Clone, Default)]
pub struct ForwardedToken(pub Option<String>);

/// Instance context extracted from the X-Instance-Id header.
/// When present, all PDT calls route to a per-instance SQLite database.
#[derive(Debug, Clone, Default)]
pub struct InstanceContext {
    pub instance_id: Option<String>,
}

impl InstanceContext {
    pub fn as_deref(&self) -> Option<&str> {
        self.instance_id.as_deref()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for InstanceContext
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let instance_id = parts
            .extensions
            .get::<InstanceContext>()
            .and_then(|c| c.instance_id.clone())
            .or_else(|| {
                parts
                    .headers
                    .get("X-Instance-Id")
                    .and_then(|h| h.to_str().ok())
                    .map(|s| s.to_string())
            });

        Ok(InstanceContext { instance_id })
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ForwardedToken
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<ForwardedToken>()
            .cloned()
            .unwrap_or_default())
    }
}

pub struct AuthenticatedUser {
    pub user_id: String,
    pub username: Option<String>,
    pub email: Option<String>,
    /// The full `extra` map from the original JWT claims (role, groups, etc.).
    pub claims_extra: Option<std::collections::HashMap<String, Value>>,
}

impl AuthenticatedUser {
    /// Build a `JwtClaims` suitable for Cedar evaluation from this user.
    ///
    /// Propagates `role` and `groups` from the original JWT `extra` map.
    /// When `claims_extra` is `None` (e.g. unit-test construction), falls back
    /// to `role = "viewer"` — a safe, least-privilege default.
    pub fn to_cedar_claims(&self) -> JwtClaims {
        let mut extra = std::collections::HashMap::new();

        match &self.claims_extra {
            Some(orig) => {
                // Propagate role if present
                if let Some(role) = orig.get("role") {
                    extra.insert("role".to_string(), role.clone());
                } else {
                    extra.insert(
                        "role".to_string(),
                        Value::String("viewer".to_string()),
                    );
                }
                // Propagate groups if present
                if let Some(groups) = orig.get("groups") {
                    extra.insert("groups".to_string(), groups.clone());
                }
                // Carry over any other fields
                for (k, v) in orig {
                    if k != "role" && k != "groups" {
                        extra.insert(k.clone(), v.clone());
                    }
                }
            }
            None => {
                // No original claims — assume least privilege
                extra.insert(
                    "role".to_string(),
                    Value::String("viewer".to_string()),
                );
            }
        }

        JwtClaims {
            sub: self.user_id.clone(),
            iss: "nghr".to_string(),
            aud: None,
            exp: i64::MAX,
            iat: None,
            email: self.email.clone(),
            name: None,
            preferred_username: self.username.clone(),
            extra,
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let claims = parts
            .extensions
            .get::<JwtClaims>()
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let extra = Some(claims.extra.clone());

        Ok(AuthenticatedUser {
            user_id: claims.sub.clone(),
            username: claims.preferred_username.clone(),
            email: claims.email.clone(),
            claims_extra: extra,
        })
    }
}

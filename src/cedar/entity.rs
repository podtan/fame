//! Cedar entity builders for NGHR
//!
//! Converts NGHR resource types and JWT claims into Cedar entities
//! for authorization evaluation.

use cedar_policy::{Entity, RestrictedExpression};
use std::collections::{HashMap, HashSet};

use pep::cedar::build_principal_uid;
use pep::oidc::types::JwtClaims;

/// Build a Cedar principal Entity from JWT claims with role and groups.
///
/// Extends PEP's base entity builder with `role` and `groups` from extra claims
/// (matching the NGHR schema's `User` entity).
///
/// Returns `Err` instead of panicking (af35f475 class, fame-side pin): a panic
/// inside the authz path crashes the request handler instead of answering a
/// clean 5xx. Mirrors PEP's own `build_principal_entity` signature. With
/// current pep the error legs are unreachable in practice (EntityId's
/// FromStr is Infallible; RestrictedExpression attrs cannot fail) — the guard
/// is the type itself: panic-by-construction is eliminated, and any future
/// pep strictness change lands as a handled 500, not a crash.
pub fn user_to_cedar_principal(claims: &JwtClaims) -> Result<Entity, pep::cedar::CedarError> {
    let uid = build_principal_uid(claims)?;
    let mut attrs: HashMap<String, RestrictedExpression> = HashMap::new();

    // Email from JWT (for Cedar policies — principal has email required)
    if let Some(ref email) = claims.email {
        attrs.insert(
            "email".to_string(),
            RestrictedExpression::new_string(email.clone()),
        );
    }

    // Add role from extra claims — handle both string and array of strings.
    //
    // Kanidm's userinfo endpoint returns `role` as a JSON array (e.g. `["admin"]`)
    // because claim maps can map multiple groups to multiple role values.
    // Some IdPs (e.g. Auth0) return a single string instead.
    if let Some(role) = claims.extra.get("role") {
        let role_str = match role {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .next(),
            _ => None,
        };
        if let Some(role_str) = role_str {
            attrs.insert(
                "role".to_string(),
                RestrictedExpression::new_string(role_str),
            );
        }
    }

    // Add groups (set from extra claims)
    if let Some(serde_json::Value::Array(groups)) = claims.extra.get("groups") {
        let set_exprs: Vec<RestrictedExpression> = groups
            .iter()
            .filter_map(|g| {
                if let serde_json::Value::String(s) = g {
                    Some(RestrictedExpression::new_string(s.clone()))
                } else {
                    None
                }
            })
            .collect();
        if !set_exprs.is_empty() {
            attrs.insert(
                "groups".to_string(),
                RestrictedExpression::new_set(set_exprs),
            );
        }
    }

    Entity::new(uid, attrs, HashSet::new()).map_err(|e| {
        pep::cedar::CedarError::EntityBuildFailed(format!("principal entity build failed: {}", e))
    })
}

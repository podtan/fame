//! Cedar entity builders for NGHR
//!
//! Converts NGHR resource types and JWT claims into Cedar entities
//! for authorization evaluation.

use cedar_policy::{Entity, RestrictedExpression};
use std::collections::{HashMap, HashSet};

use pep::cedar::build_principal_uid;
use pep::oidc::types::JwtClaims;

/// Build a Cedar principal Entity from JWT claims UNION server-resolved
/// PDT-native groups (3ad59ed3 — the resolve_instance_authz equivalent,
/// PDT Phase 1b pattern).
///
/// The token's `groups` claim and the PDT-resolved set UNION into the
/// principal's `groups` attribute (deduped + sorted for deterministic
/// entity bytes). The drain (bf70097a) stops the token's groups claim and
/// the resolved set carries `principal.groups`; with the claim present the
/// union is a superset of today's claims-only behavior (no regression).
/// Resolve failures degrade to the token groups at the CALL SITE (never
/// zero while either source has material — union semantics, 147a17fc).
///
/// Returns `Err` instead of panicking (af35f475 class, fame-side pin): a
/// panic inside the authz path crashes the request handler instead of
/// answering a clean 5xx. With current pep the error legs are unreachable
/// in practice (EntityId's FromStr is Infallible; RestrictedExpression
/// attrs cannot fail) — the guard is the type itself.
pub fn user_to_cedar_principal_with_groups(
    claims: &JwtClaims,
    extra_groups: &[String],
) -> Result<Entity, pep::cedar::CedarError> {
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

    // Groups: the token's groups claim UNION the server-resolved PDT-native
    // set, deduped + sorted (deterministic entity bytes — the same shape
    // PDT's evaluate_permission hands its principal builder).
    let mut effective: Vec<String> = Vec::new();
    if let Some(serde_json::Value::Array(groups)) = claims.extra.get("groups") {
        for g in groups {
            if let serde_json::Value::String(s) = g {
                effective.push(s.clone());
            }
        }
    }
    effective.extend(extra_groups.iter().cloned());
    effective.sort();
    effective.dedup();
    if !effective.is_empty() {
        let set_exprs: Vec<RestrictedExpression> = effective
            .into_iter()
            .map(RestrictedExpression::new_string)
            .collect();
        attrs.insert(
            "groups".to_string(),
            RestrictedExpression::new_set(set_exprs),
        );
    }

    Entity::new(uid, attrs, HashSet::new()).map_err(|e| {
        pep::cedar::CedarError::EntityBuildFailed(format!("principal entity build failed: {}", e))
    })
}

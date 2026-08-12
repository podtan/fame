//! Cedar enforcement logic for NGHR
//!
//! Evaluates authorization requests against Cedar policies.
//! Unlike PDT, NGHR has no grandfathering — ALL resources go through Cedar.

use axum::http::StatusCode;
use cedar_policy::{Context, Entities, Request};
use pep::cedar::CedarAuthorizer;
use pep::oidc::types::JwtClaims;

use crate::auth::AuthenticatedUser;

/// Extract JwtClaims from AuthenticatedUser for Cedar evaluation.
pub fn extract_claims_for_cedar(user: &AuthenticatedUser) -> JwtClaims {
    user.to_cedar_claims()
}

/// Check if a user can perform an action on a resource type.
///
/// Returns `Ok(())` if allowed, `Err(StatusCode::FORBIDDEN)` if denied.
/// Returns `Err(StatusCode::INTERNAL_SERVER_ERROR)` on evaluation errors.
pub fn check_permission(
    authorizer: &CedarAuthorizer,
    claims: &JwtClaims,
    action: &str,
    entity_type: &str,
    entity_id: &str,
) -> Result<(), StatusCode> {
    let principal = pep::cedar::build_principal_uid(claims).map_err(|e| {
        tracing::error!("Cedar principal build failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let action_uid = pep::cedar::build_action_uid(action).map_err(|e| {
        tracing::error!("Cedar action build failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let resource =
        pep::cedar::entity::ResourceInfo::new(entity_type, entity_id)
            .to_cedar_uid()
            .map_err(|e| {
                tracing::error!("Cedar resource build failed: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    // Build principal entity with role/groups for attribute-based policies
    let principal_entity = crate::cedar::entity::user_to_cedar_principal(claims);
    let entities = Entities::from_entities([principal_entity], None).map_err(|e| {
        tracing::error!("Cedar entities build failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let request = Request::new(principal, action_uid, resource, Context::empty(), None).map_err(
        |e| {
            tracing::error!("Cedar request build failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        },
    )?;

    tracing::debug!(
        "Cedar eval: action={}, resource={}:{}",
        action, entity_type, entity_id
    );

    let response = authorizer.is_allowed_with_entities(&request, &entities);

    // Log detailed response for debugging
    if response.has_errors() {
        tracing::error!(
            "Cedar evaluation ERRORS for {} {} on {}:{} for user {}: {:?}",
            action, entity_type, entity_type, entity_id, claims.sub,
            response.errors()
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if response.allowed() {
        Ok(())
    } else {
        tracing::warn!(
            "Cedar denied {} {} on {}:{} for user {}",
            action,
            entity_type,
            entity_type,
            entity_id,
            claims.sub
        );
        Err(StatusCode::FORBIDDEN)
    }
}

/// Check if user can perform action — returns bool (for filtering).
pub fn is_allowed(
    authorizer: &CedarAuthorizer,
    claims: &JwtClaims,
    action: &str,
    entity_type: &str,
    entity_id: &str,
) -> bool {
    check_permission(authorizer, claims, action, entity_type, entity_id).is_ok()
}

//! Cedar enforcement logic for NGHR
//!
//! Evaluates authorization requests against Cedar policies.
//! Unlike PDT, NGHR has no grandfathering — ALL resources go through Cedar.

use axum::http::StatusCode;
use cedar_policy::{Context, Entities, Entity, Request, RestrictedExpression};
use pep::cedar::CedarAuthorizer;
use pep::oidc::types::JwtClaims;

use crate::auth::AuthenticatedUser;

/// Extract JwtClaims from AuthenticatedUser for Cedar evaluation.
pub fn extract_claims_for_cedar(user: &AuthenticatedUser) -> JwtClaims {
    user.to_cedar_claims()
}

/// Metadata key on AgentIdentity assets holding the workspace's Kanidm
/// admin group — the workspace-scoping anchor for Cedar (0.2.0).
pub const ADMIN_GROUP_METADATA_KEY: &str = "admin_group";

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

/// Check an action against a resource that carries string attributes
/// (workspace-scoped authorization, 0.2.0).
///
/// The plain `check_permission` builds a bare resource UID — fine for
/// role-only policies, useless for policies that reference resource
/// attributes (`resource.admin_group`). This variant constructs a full
/// resource Entity carrying the given attributes so attribute-based
/// conditions can evaluate.
pub fn check_permission_scoped(
    authorizer: &CedarAuthorizer,
    claims: &JwtClaims,
    action: &str,
    entity_type: &str,
    entity_id: &str,
    resource_attrs: &[(String, String)],
) -> Result<(), StatusCode> {
    let principal = pep::cedar::build_principal_uid(claims).map_err(|e| {
        tracing::error!("Cedar principal build failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let action_uid = pep::cedar::build_action_uid(action).map_err(|e| {
        tracing::error!("Cedar action build failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let resource_uid = pep::cedar::entity::ResourceInfo::new(entity_type, entity_id)
        .to_cedar_uid()
        .map_err(|e| {
            tracing::error!("Cedar resource build failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let principal_entity = crate::cedar::entity::user_to_cedar_principal(claims);

    let attrs: std::collections::HashMap<String, RestrictedExpression> = resource_attrs
        .iter()
        .cloned()
        .map(|(k, v)| (k, RestrictedExpression::new_string(v)))
        .collect();
    let resource_entity =
        Entity::new(resource_uid.clone(), attrs, std::collections::HashSet::new()).map_err(|e| {
            tracing::error!("Cedar resource entity build failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let entities = Entities::from_entities([principal_entity, resource_entity], None).map_err(|e| {
        tracing::error!("Cedar entities build failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let request = Request::new(principal, action_uid, resource_uid, Context::empty(), None)
        .map_err(|e| {
            tracing::error!("Cedar request build failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let response = authorizer.is_allowed_with_entities(&request, &entities);

    if response.has_errors() {
        tracing::error!(
            "Cedar evaluation ERRORS for {} on {}:{} for user {}: {:?}",
            action, entity_type, entity_id, claims.sub,
            response.errors()
        );
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    if response.allowed() {
        Ok(())
    } else {
        tracing::warn!(
            "Cedar denied {} on {}:{} for user {} (scoped: {} attrs)",
            action, entity_type, entity_id, claims.sub,
            resource_attrs.len()
        );
        Err(StatusCode::FORBIDDEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthenticatedUser;

    /// Build a runtime-shaped CedarConfig: embedded policy/schema (the same
    /// strings main.rs compiles in via include_str!), no filesystem, no store.
    /// An empty test temp-dir would resolve filesystem-first and load
    /// whatever stale files exist there.
    fn test_cedar_config() -> pep::cedar::CedarConfig {
        use pep::cedar::config::DefaultDecision;
        pep::cedar::CedarConfig {
            policy_path: std::path::PathBuf::from("/nonexistent/fame-test-policies"),
            schema_path: None,
            entities_path: None,
            default_decision: DefaultDecision::Deny,
            validate_on_load: true,
            policy_store_url: None,
            policy_store_token: None,
            embedded_policy: Some(include_str!("../../policies/rbac.cedar")),
            embedded_schema: Some(include_str!("../../policies/schema.cedarschema")),
        }
    }

    fn claims_with_role(role: Option<&str>) -> JwtClaims {
        let user = AuthenticatedUser {
            user_id: format!("user-{}", role.unwrap_or("norole")),
            username: Some("tester".to_string()),
            email: Some("tester@example.com".to_string()),
            claims_extra: role.map(|r| {
                let mut extra = std::collections::HashMap::new();
                extra.insert(
                    "role".to_string(),
                    serde_json::Value::String(r.to_string()),
                );
                extra
            }),
        };
        user.to_cedar_claims()
    }

    /// The gate: with the empty-namespace + attribute-role policy, Cedar must
    /// initialize from the embedded sources and decide ALLOW for an admin
    /// principal and DENY for everyone else. This is the regression test for
    /// the fail-open bug (v0.1.0–0.1.1: schema never parsed → authorizer
    /// None → every check silently skipped).
    #[test]
    fn cedar_enforces_admin_vs_viewer() {
        let authorizer = pep::cedar::CedarAuthorizer::new(test_cedar_config())
            .expect("Cedar must initialize from embedded policy/schema");
        let resource_type = crate::entity_config::slug_to_cedar_type("semantic-memory");

        // Positive control: admin can create semantic memories
        let admin = claims_with_role(Some("admin"));
        assert_eq!(
            check_permission(
                &authorizer,
                &admin,
                "CreateSemanticMemory",
                &resource_type,
                "<_>"
            ),
            Ok(())
        );

        // Negative control: viewer denied
        let viewer = claims_with_role(Some("viewer"));
        assert_eq!(
            check_permission(&authorizer, &viewer, "CreateSemanticMemory", &resource_type, "<_>"),
            Err(StatusCode::FORBIDDEN)
        );

        // Negative control: no role attribute at all → denied (not a 500:
        // the policy guards with `has role` instead of accessing it)
        let norole = claims_with_role(None);
        assert_eq!(
            check_permission(&authorizer, &norole, "CreateSemanticMemory", &resource_type, "<_>"),
            Err(StatusCode::FORBIDDEN)
        );
    }

    /// Roles must scope capabilities, not just gate them: an agent principal
    /// may write its own memory kinds but must not delete anything.
    #[test]
    fn cedar_scopes_agent_role() {
        let authorizer = pep::cedar::CedarAuthorizer::new(test_cedar_config())
            .expect("Cedar must initialize from embedded policy/schema");

        let agent = claims_with_role(Some("agent"));

        // Allowed for agent: create episodic
        assert_eq!(
            check_permission(
                &authorizer,
                &agent,
                "CreateEpisodicMemory",
                &crate::entity_config::slug_to_cedar_type("episodic-memory"),
                "<_>"
            ),
            Ok(())
        );

        // Denied for agent: delete semantic
        assert_eq!(
            check_permission(
                &authorizer,
                &agent,
                "DeleteSemanticMemory",
                &crate::entity_config::slug_to_cedar_type("semantic-memory"),
                "<_>"
            ),
            Err(StatusCode::FORBIDDEN)
        );

        // Denied for agent: identity creation is conductor/admin-only
        assert_eq!(
            check_permission(
                &authorizer,
                &agent,
                "CreateAgentIdentity",
                &crate::entity_config::slug_to_cedar_type("agent-identity"),
                "<_>"
            ),
            Err(StatusCode::FORBIDDEN)
        );
    }

    /// 0.1.3 regression test: the workspace-owner path. TOCPI forwards the
    /// creating user's token when it stores an agent's identity charter in
    /// fame; since Cedar enforcement went live (0.1.2) that call returned
    /// 403 for role=user — agents were born with a silently missing
    /// charter. user must now be permitted identity CRUD + memory View,
    /// while staying denied for memory writes and deletes.
    #[test]
    fn cedar_user_role_identity_permits() {
        let authorizer = pep::cedar::CedarAuthorizer::new(test_cedar_config())
            .expect("Cedar must initialize from embedded policy/schema");

        let user = claims_with_role(Some("user"));
        let identity = crate::entity_config::slug_to_cedar_type("agent-identity");

        // The regression: agent creation via TOCPI must work again.
        // 0.2.0: user identity Create/Edit are workspace-scoped — they pass
        // only with the caller's group on the resource; the unscoped form
        // here asserts View still works globally and scoped form below
        // asserts the workspace-owner path end-to-end.
        let ws_group = "ws-demo-admins".to_string();
        let ws_member = claims_with_role_and_groups("user", &[&ws_group]);
        assert_eq!(
            check_permission_scoped(
                &authorizer,
                &ws_member,
                "CreateAgentIdentity",
                &identity,
                "<_>",
                &[(ADMIN_GROUP_METADATA_KEY.to_string(), ws_group.clone())]
            ),
            Ok(())
        );
        assert_eq!(
            check_permission(
                &authorizer,
                &user,
                "ViewAgentIdentity",
                &identity,
                "some-agent-id"
            ),
            Ok(())
        );

        // Memory inspection allowed (debugging), memory writes denied
        let memory = crate::entity_config::slug_to_cedar_type("semantic-memory");
        assert_eq!(
            check_permission(
                &authorizer,
                &user,
                "ViewSemanticMemory",
                &memory,
                "some-memory-id"
            ),
            Ok(())
        );
        assert_eq!(
            check_permission(&authorizer, &user, "CreateSemanticMemory", &memory, "<_>"),
            Err(StatusCode::FORBIDDEN)
        );

        // Deletes stay admin-only for everyone
        assert_eq!(
            check_permission(
                &authorizer,
                &user,
                "DeleteAgentIdentity",
                &identity,
                "some-agent-id"
            ),
            Err(StatusCode::FORBIDDEN)
        );
    }

    fn claims_with_role_and_groups(role: &str, groups: &[&str]) -> JwtClaims {
        let user = AuthenticatedUser {
            user_id: format!("user-{}", role),
            username: Some("tester".to_string()),
            email: Some("tester@example.com".to_string()),
            claims_extra: Some({
                let mut extra = std::collections::HashMap::new();
                extra.insert(
                    "role".to_string(),
                    serde_json::Value::String(role.to_string()),
                );
                extra.insert(
                    "groups".to_string(),
                    serde_json::Value::Array(
                        groups.iter().map(|g| serde_json::Value::String(g.to_string())).collect(),
                    ),
                );
                extra
            }),
        };
        user.to_cedar_claims()
    }

    /// 0.2.0: identity create/edit are workspace-scoped via the resource's
    /// admin_group attribute. A user may manage a charter iff their groups
    /// claim contains the workspace's admin group. Admin remains unscoped.
    #[test]
    fn cedar_workspace_scoped_identity() {
        let authorizer = pep::cedar::CedarAuthorizer::new(test_cedar_config())
            .expect("Cedar must initialize from embedded policy/schema");

        let identity = crate::entity_config::slug_to_cedar_type("agent-identity");
        let group = "ws-4b31ec2d-3c10-4b5d-90b5-9e92ccc77cef-admins".to_string();
        let scoped = vec![(ADMIN_GROUP_METADATA_KEY.to_string(), group.clone())];

        // Workspace member: allowed
        let member = claims_with_role_and_groups("user", &[&group]);
        assert_eq!(
            check_permission_scoped(
                &authorizer,
                &member,
                "CreateAgentIdentity",
                &identity,
                "<_>",
                &scoped
            ),
            Ok(())
        );
        assert_eq!(
            check_permission_scoped(
                &authorizer,
                &member,
                "EditAgentIdentity",
                &identity,
                "agent-abc",
                &scoped
            ),
            Ok(())
        );

        // Outsider (different workspace group): denied
        let outsider = claims_with_role_and_groups("user", &["ws-other-admins"]);
        assert_eq!(
            check_permission_scoped(
                &authorizer,
                &outsider,
                "EditAgentIdentity",
                &identity,
                "agent-abc",
                &scoped
            ),
            Err(StatusCode::FORBIDDEN)
        );

        // No groups claim at all: denied
        let groupless = claims_with_role(Some("user"));
        assert_eq!(
            check_permission_scoped(
                &authorizer,
                &groupless,
                "CreateAgentIdentity",
                &identity,
                "<_>",
                &scoped
            ),
            Err(StatusCode::FORBIDDEN)
        );

        // Resource without admin_group (no header at create): user denied,
        // admin still allowed via unscoped permit
        let admin = claims_with_role(Some("admin"));
        assert_eq!(
            check_permission_scoped(
                &authorizer,
                &groupless,
                "CreateAgentIdentity",
                &identity,
                "<_>",
                &[]
            ),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            check_permission_scoped(
                &authorizer,
                &admin,
                "CreateAgentIdentity",
                &identity,
                "<_>",
                &[]
            ),
            Ok(())
        );
    }
}

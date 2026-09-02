// Fame Cedar RBAC policy — empty namespace, attribute-based roles (v0.1.2).
//
// Roles travel as the `role` attribute on the principal User entity, built
// from JWT claims in cedar::entity::user_to_cedar_principal. The entities set
// handed to the evaluator contains ONLY the principal (no Role hierarchy
// entities, no parents), so a membership form like `principal in
// Role::"admin"` would never match at runtime. Use the attribute form — the
// same one NGHR runs in production:
//     principal has role && principal.role == "admin"
//
// The `has` guard is required: accessing a missing attribute is an evaluation
// error (→ HTTP 500), whereas `has` simply evaluates false.
//
// Default decision is DENY (pep CedarConfig), so anything not permitted here
// is rejected with 403.
//
// Enforcement sites (see entity_handlers.rs): create + edit/status-update.
// Reads remain gated by the auth layer and per-agent X-Instance-Id isolation.

// Admin role — full access to all memory and identity operations
permit (
    principal,
    action in [
        Action::"CreateSemanticMemory",
        Action::"ViewSemanticMemory",
        Action::"EditSemanticMemory",
        Action::"DeleteSemanticMemory",
        Action::"CreateEpisodicMemory",
        Action::"ViewEpisodicMemory",
        Action::"EditEpisodicMemory",
        Action::"DeleteEpisodicMemory",
        Action::"CreateProceduralMemory",
        Action::"ViewProceduralMemory",
        Action::"EditProceduralMemory",
        Action::"DeleteProceduralMemory",
        Action::"CreateAgentIdentity",
        Action::"ViewAgentIdentity",
        Action::"EditAgentIdentity",
        Action::"DeleteAgentIdentity",
    ],
    resource
)
when { principal has role && principal.role == "admin" };

// Agent role — can create and view memories, bootstrap-create identities,
// view identities, and edit ONLY its own identity (v0.3.5: edit is
// resource-scoped — the identity's admin_group, stamped mem-<agent> at
// create by tocpi v0.6.3, must be in the caller's groups; the cross-agent
// edit deny is now policy, not accident).
permit (
    principal,
    action in [
        Action::"CreateSemanticMemory",
        Action::"ViewSemanticMemory",
        Action::"EditSemanticMemory",
        Action::"CreateEpisodicMemory",
        Action::"ViewEpisodicMemory",
        Action::"EditEpisodicMemory",
        Action::"CreateProceduralMemory",
        Action::"ViewProceduralMemory",
        Action::"EditProceduralMemory",
        Action::"ViewAgentIdentity",
        Action::"CreateAgentIdentity",
    ],
    resource
)
when { principal has role && principal.role == "agent" };

permit (
    principal,
    action == Action::"EditAgentIdentity",
    resource
)
when {
    principal has role
    && principal.role == "agent"
    && principal has groups
    && resource has admin_group
    && principal.groups.contains(resource.admin_group)
};

// Service role — tocpi control-plane bootstrap: may create, view, and
// update agent identities on behalf of the platform (charter provisioning).
// Memory ops, deletes, and admin surfaces stay denied (default deny).
permit (
    principal,
    action in [
        Action::"CreateAgentIdentity",
        Action::"ViewAgentIdentity",
        Action::"EditAgentIdentity",
    ],
    resource
)
when { principal has role && principal.role == "service" };

// Developer role — can view and create memories for debugging, view identity
permit (
    principal,
    action in [
        Action::"ViewSemanticMemory",
        Action::"ViewEpisodicMemory",
        Action::"ViewProceduralMemory",
        Action::"CreateSemanticMemory",
        Action::"CreateEpisodicMemory",
        Action::"CreateProceduralMemory",
        Action::"ViewAgentIdentity",
    ],
    resource
)
when { principal has role && principal.role == "developer" };

// User role — human workspace owners acting via TOCPI (agent create/edit
// forwards their token here). They manage agent identity charters; they do
// NOT write agent memories (memory writes are agent-attributed — humans
// get read access for debugging/inspection only).
//
// 0.2.0: identity Create/Edit are workspace-scoped — the AgentIdentity
// resource carries an `admin_group` attribute (the workspace's Kanidm
// admin group, set by TOCPI at create and persisted in the asset metadata
// for later edits). A user may only manage the charter if their `groups`
// claim contains that group. View stays global (inspection); deletes stay
// admin-only.
permit (
    principal,
    action in [
        Action::"ViewAgentIdentity",
        Action::"ViewSemanticMemory",
        Action::"ViewEpisodicMemory",
        Action::"ViewProceduralMemory",
    ],
    resource
)
when { principal has role && principal.role == "user" };

permit (
    principal,
    action in [
        Action::"CreateAgentIdentity",
        Action::"EditAgentIdentity",
    ],
    resource
)
when {
    principal has role
    && principal.role == "user"
    && principal has groups
    && resource has admin_group
    && principal.groups.contains(resource.admin_group)
};

// Conductor role (L3) — can create, view, and edit identity for any agent.
// This is the "upper layer agent" that births and evolves other agents.
permit (
    principal,
    action in [
        Action::"CreateAgentIdentity",
        Action::"ViewAgentIdentity",
        Action::"EditAgentIdentity",
        Action::"ViewSemanticMemory",
        Action::"ViewEpisodicMemory",
        Action::"ViewProceduralMemory",
    ],
    resource
)
when { principal has role && principal.role == "conductor" };

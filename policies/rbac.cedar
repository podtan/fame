// Fame Cedar RBAC Policy
//
// Default policy: authenticated users can manage their own agent memories.
// Agent isolation is enforced at the application level via the agent:{id} tag,
// but Cedar provides an additional authorization layer based on user roles/groups.

// Admin role — full access to all memory operations
permit (
    principal in Podtan::Role::"admin",
    action in [
        Podtan::Action::"CreateSemanticMemory",
        Podtan::Action::"ViewSemanticMemory",
        Podtan::Action::"EditSemanticMemory",
        Podtan::Action::"DeleteSemanticMemory",
        Podtan::Action::"CreateEpisodicMemory",
        Podtan::Action::"ViewEpisodicMemory",
        Podtan::Action::"EditEpisodicMemory",
        Podtan::Action::"DeleteEpisodicMemory",
        Podtan::Action::"CreateProceduralMemory",
        Podtan::Action::"ViewProceduralMemory",
        Podtan::Action::"EditProceduralMemory",
        Podtan::Action::"DeleteProceduralMemory",
    ],
    resource
);

// Agent role — can create and view memories
permit (
    principal in Podtan::Role::"agent",
    action in [
        Podtan::Action::"CreateSemanticMemory",
        Podtan::Action::"ViewSemanticMemory",
        Podtan::Action::"EditSemanticMemory",
        Podtan::Action::"CreateEpisodicMemory",
        Podtan::Action::"ViewEpisodicMemory",
        Podtan::Action::"EditEpisodicMemory",
        Podtan::Action::"CreateProceduralMemory",
        Podtan::Action::"ViewProceduralMemory",
        Podtan::Action::"EditProceduralMemory",
    ],
    resource
);

// Developer role — can view and create memories for debugging
permit (
    principal in Podtan::Role::"developer",
    action in [
        Podtan::Action::"ViewSemanticMemory",
        Podtan::Action::"ViewEpisodicMemory",
        Podtan::Action::"ViewProceduralMemory",
        Podtan::Action::"CreateSemanticMemory",
        Podtan::Action::"CreateEpisodicMemory",
        Podtan::Action::"CreateProceduralMemory",
    ],
    resource
);

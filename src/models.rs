//! OpenAPI spec definition using utoipa.
//!
//! Fame has no hardcoded entity handlers — all paths and schemas are generated
//! at runtime from TOML entity configs by [`crate::openapi_gen`].

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Fame — Facade for Agent Memory",
        description = "Memory facade over PDT. Provides semantic, episodic, and procedural memory per agent.\n\nAgent identity is passed via X-Instance-Id header (injected by Aether). The agent never sees or passes agent_id — the URL path to Fame (https://fame.aether.tanbal.ir/{agent_id}/sse) tells Aether which agent to route to, and Aether injects X-Instance-Id.",
        version = "0.1.0",
        contact(name = "Podtan Team"),
        license(name = "MIT OR Apache-2.0"),
    ),
    paths(),
    components(schemas()),
    tags(
        (name = "semantic-memory", description = "Long-term facts and knowledge"),
        (name = "episodic-memory", description = "Specific events and experiences"),
        (name = "procedural-memory", description = "How-to procedures and skills"),
    ),
)]
pub struct ApiDoc;

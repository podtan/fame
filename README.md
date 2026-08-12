# Fame

**F**acade for **A**gent **Me**mory — a config-driven entity server providing semantic, episodic, and procedural memory types as PDT assets, scoped per-agent via `X-Instance-Id` header.

## What is Fame?

Fame is a thin facade over [PDT](https://github.com/Podtan/pdt). It maps cognitive memory concepts (semantic, episodic, procedural) to PDT assets and provides per-agent memory isolation using tags.

Built on the same dynamic entity engine as [NGHR](https://github.com/Podtan/nghr) — no hardcoded entity handlers. All three memory types are defined as TOML config files and served by a generic entity handler.

## How It Works

```
Trustee Agent
     │ MCP tools (via Aether)
     ▼
Aether (OpenAPI → MCP converter)
     │ POST /api/v1/semantic-memories
     │ Header: X-Instance-Id: {agent_id}
     ▼
Fame ← validates JWT (PEP OIDC)
     │ ← authorizes (PEP Cedar)
     │ ← injects tag: agent:{agent_id}
     ▼
PDT (stores asset with tags: type:agent-memory, memory-type:semantic, agent:{id})
```

The agent never sees or passes `agent_id`. Aether injects it via `X-Instance-Id` header automatically.

## Memory Types

| Type | Tag | Purpose |
|------|-----|---------|
| Semantic | `memory-type:semantic` | Long-term facts and knowledge |
| Episodic | `memory-type:episodic` | Specific events and experiences |
| Procedural | `memory-type:procedural` | How-to procedures and skills |

Every memory asset gets: `type:agent-memory` + `memory-type:{type}` + `agent:{agent_id}` + domain tags.

## Quick Start

```bash
# Copy and edit config
cp fame.env.example .env

# Run
cargo run
```

Fame will be available at `http://localhost:8628` with Swagger UI at `/swagger-ui`.

## Configuration

| Env Var | Default | Description |
|---------|---------|-------------|
| `FAME_HOST` | `0.0.0.0` | Listen address |
| `FAME_PORT` | `8628` | Listen port |
| `FAME_PDT_URL` | `http://localhost:8080` | PDT API URL |
| `FAME_ENTITY_DIR` | `entities` | Entity TOML config directory |
| `FAME_CONFIG_PATH` | - | Path to TOML config file (overrides env) |

## License

MIT OR Apache-2.0

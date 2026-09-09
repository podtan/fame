# Fame

**F**acade for **A**gent **M**emory — a config-driven entity server over
[PDT](https://github.com/Podtan/pdt). Memory types and agent identities are defined as
TOML configs and served by one generic entity engine: no hardcoded handlers, no schema
migrations when you add an entity.

Entities are stored as PDT assets, scoped per caller via the `X-Instance-Id` header, and
authorized per request by [Cedar](https://www.cedarpolicy.com/) policies.

## Entities

| Entity | Slug | Default tags | Purpose |
|--------|------|--------------|---------|
| Semantic memory | `semantic-memory` | `type:agent-memory`, `memory-type:semantic` | Long-term facts and knowledge |
| Episodic memory | `episodic-memory` | `type:agent-memory`, `memory-type:episodic` | Events and experiences |
| Procedural memory | `procedural-memory` | `type:agent-memory`, `memory-type:procedural` | How-to procedures and skills |
| Agent identity | `agent-identity` | `type:agent-identity` | An agent's charter / self-model |

Entity behavior lives in `entities/*.toml`: slug, default tags, Cedar actions, field
mappings (text / markdown / tag_array / enum / metadata / date), and list/detail view
layouts. Adding an entity is a config change, not a code change — the list/get engine
filters by the entity's full default-tag set.

## API

```
GET    /api/v1/{slug}s                    # list (X-Instance-Id scoped)
POST   /api/v1/{slug}s                    # create
GET    /api/v1/{slug}s/{id}               # get
PATCH  /api/v1/{slug}s/{id}/status        # status update
PATCH  /api/v1/{slug}s/{id}/content       # content update (agent-identity only)
GET    /health                            # {status, service, version}
GET    /swagger-ui                        # OpenAPI UI
```

## Authentication and authorization

Fame validates OIDC bearer tokens through
[pep](https://crates.io/crates/pep) (JWKS signature, expiry, audience, optional userinfo
enrichment) and enforces `policies/rbac.cedar` per request with a **default-deny**
posture. Roles travel as a `role` attribute on the principal, built from JWT claims.

Authorization is attribute-based and defined per entity in the TOML configs — e.g. an
agent may edit its own identity but not another's: the identity is stamped at birth with
an `admin_group` (`mem-<agent>`), and the edit permit requires that group in the caller's
claims. Memory assets are likewise born with an `auth_context` so their owner group can
read them.

Failure contract: a rejected token answers `401` with
`WWW-Authenticate: Bearer error="invalid_token"` and a retryable JSON body — never a
silent downgrade to a role-less principal.

> ⚠️ **`dev_mode = true` injects admin claims into any request that carries no bearer
> token — including when auth is enabled.** Never enable it outside local development.

## Configuration

Loaded from `fame.toml` (path override: `FAME_CONFIG_PATH`) with `FAME_`-prefixed
environment overrides (nest with `__`, e.g. `FAME_AUTH__ENABLED=true`):

```toml
host = "0.0.0.0"
port = 8628
pdt_url = "http://localhost:8090"     # PDT API base URL
entity_dir = "entities"

[auth]
enabled = false                        # OIDC enforcement
issuer_url = "http://localhost:8080"
dev_mode = true                        # see warning above

[cedar]
enabled = true                         # boot fails loudly if init fails while enabled
policy_path = "./policies"
schema_path = "./policies/schema.cedarschema"
validate_on_load = true
```

`FAME_HOST`, `FAME_PORT`, `FAME_PDT_URL`, `FAME_ENTITY_DIR` cover the top level; `CEDAR_*`
and `AUTH_*` environment variables provide fallbacks for the sections above. See
`fame.env.example`.

## Development

```bash
cargo test                     # default tier: in-process mock PDT speaking PDT's wire format
FAME_IT_PDT_BIN=/path/to/pdt cargo test   # real-PDT integration tier
scripts/check_policies.sh      # validate Cedar policy + schema with cedar-policy-cli
```

The two-tier harness exists because wire details matter: the mock persists `auth_context`
exactly like real PDT at create, so test-green means wire-green. Policies are compiled
into the binary via `include_str!` — `scripts/check_policies.sh` validates the on-disk
sources exactly as a release would.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

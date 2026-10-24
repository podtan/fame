# Changelog

All notable changes to Fame are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.16] - 2026-10-24

### Fixed
- List rows are **slim by default** (owner-directed): a list row is now
  `{id, title(≤80 with "…"), status, updated_at(minute-precision UTC)}` —
  ~195 bytes vs ~422. `created_at`, family metadata
  (consolidated/outcome/source), and the long title are GET-by-id
  territory; `verbose=true` restores the full compact row (200-char
  title, created_at, family metadata, full-precision timestamps).
  Non-UTC timestamps are converted to UTC before the short form — never
  mislabeled; unparseable stamps pass through verbatim.
- List endpoints are **bounded by default** (owner-measured, 2026-10-24):
  a naive list call returned a 380-row store (~66K tokens added on an
  8.9K baseline — a natural-language "ask for less" never reached the
  wire, and opt-in paging can't stop what callers don't opt into). Every
  list response now carries the newest 20 rows by default
  (`DEFAULT_LIST_LIMIT`, matching PDT's own search default), with the
  TRUE `total` + `returned` always present — honest paging, never the
  lying class (b3209637). `limit` is an override for deliberate full
  inventories. The cut happens after the updated_at-DESC sort, so a
  bounded page keeps the newest rows restore rituals actually read.
  Applies to all four generic list surfaces; the OpenAPI spec (and the
  aether-derived MCP tool) carries the `limit` + `verbose` params and
  the `returned` field.

## [0.3.15] - 2026-10-24

### Fixed
- List endpoints are **compact by default** (aed50913, owner-reported):
  one `fame_list_episodic-memorys` call measured 117,277 bytes / ~29K
  tokens on unbounded ledger titles — big-memory agents flooded ~100K
  tokens per list call without reading a single memory. List views now
  carry a **truncated title (200 chars, `…` marks the cut)**; the full
  title + content live only on GET by id. The cut is honest, never
  silent, and the read surface is untouched.
  Guards: `list_title_truncates_at_200_chars`,
  `list_title_short_titles_pass_through_untouched`,
  `detail_view_keeps_full_title`,
  `multibyte_titles_cut_at_char_boundary_never_panic`.

## [0.3.14] - 2026-10-20

### Fixed
- List projections stopped **fabricating `updated_at` into `created_at`**
  (bbb4882c): the true birth-stamp now flows from pdt v0.3.10 search
  results; on older PDT tiers the field is omitted — absent, never a lie.
  Guard: `created_at_projects_true_value_and_never_fabricates`.
- Entity list endpoints are **cursor-paginated** (b3209637): `search_by_tag`
  walks PDT's keyset cursor until exhaustion — the bounded window is gone,
  `total` is the complete matching set, and the oldest records can no longer
  be silently evicted while staying GET-able by id. A 100-page safety cap
  logs loudly if ever hit (a misbehaving backend must not loop the cursor).
  This closes the interim mitigation left open in 0.3.10.
  Guard: `paged_search_walks_the_cursor_to_exhaustion` (two-tier).
- Entity GET **validates id shape at the boundary** (91ef0f8b): malformed
  (non-UUID) ids answer 422 validation error — never 404-absent, which
  masked caller transcription errors as "genuinely missing".
  Guard: `malformed_id_is_422_not_404`.

Gates at ship: build clean; 50/50 tests; fmt clean; warning delta zero.
Rollback pin: v0.3.13 @ 72289e8.

## [0.3.13] - 2026-10-20

### Fixed
- The memory status lifecycle was **write-only** (NGHR issues 80c2544e +
  6abfa0b5): `update_episodic-memory_status` / `update_procedural-memory_status`
  / `update_semantic-memory_status` validated nothing, wrote a `status` tag to
  PDT, returned 200 with `updated_at` bumped — and no surface ever projected
  the tag back. GET/list bodies carried no `status` field and the
  `?status=` filter matched nothing under any value (0 results for
  `archived`, `active`, `Archived` against a 147-entity list). Root cause:
  the three memory entity configs declared no `status` field, so the
  config-driven projection never emitted one while the update endpoint
  wrote it anyway — a write the API accepted and stored unobservably.
- The three memory configs now declare the lifecycle: `status` enum
  (`active` | `archived`, default `active`, mapped to the `status` tag),
  surfaced in list + detail views. Declared field defaults now fill absent
  mapped values in both projection paths, so never-statused memories
  project `active` and the `?status=` filter matches the full set —
  positive AND negative controls hold (an untouched entity is absent from
  the `archived` filter).
- Status validation is now live for memories (invalid values → 400 with
  `allowed_values`), the update response echoes the stored status, and the
  served spec documents the accepted vocabulary on both the list `?status=`
  param and the PATCH body schema.
- The instance-scoped `update_asset_tag` was **post-only** — it never
  removed the existing tag of the same category, so every status update
  ACCUMULATED a duplicate tag while `get_tag` returned the first match:
  the original value won forever and the update was a silent no-op at the
  projection layer (wire-proven 2026-10-20: an update left BOTH `active`
  and `archived` tags and read-back kept showing the old value). The
  update is now delete-then-post on both PDT clients, and every delete/
  post step checks the HTTP status — `send()` alone never raises on
  4xx/5xx, so a discarded failure re-created the duplicate silently.

## [0.3.12] - 2026-10-05

### Fixed
- Status updates on an absent asset reported the opaque reqwest message
  ("error decoding response body") instead of the truth: the instance-tier
  tag-update decoded PDT's final GET response WITHOUT a status check, so a
  PDT 404 (asset absent in the routed store) was parsed as an asset and
  failed as a decode error — misdirecting the 0473c444 investigation for a
  full round. The update path now uses the discriminating fetch: absence is
  a clean 404, everything else stays a loud 500.
- Entity GET absence (PDT 404) now journals slug/id/instance — listed-but-
  unGETable records start from a log line, not a mystery.

## [0.3.11] - 2026-10-04

### Fixed
- A live, correctly-banded memory could read as **404** (GET) and **500
  "error decoding response body"** (status update) while its storage row
  was verifiably intact (NGHR issue 0473c444). Root cause: fame's strict
  `PdtAsset` decode rejected payloads whose optional fields arrived null
  (timestamps, metadata, tag ids, auth sub-fields — the sqlite vs mongo
  storage tiers and older writers differ here), and the GET handler mapped
  EVERY fetch error — including decode failures — to NOT_FOUND, masking
  live data behind a bug.
- `PdtAsset`/`PdtTag`/`AuthContext` decoding is now null-tolerant: null or
  missing optional fields decode to defaults, matching the list tier's
  tolerance (envelope fidelity — the same asset reads the same through
  every surface). Genuinely malformed payloads (missing `_id`, wrong-typed
  `auth_context`) still fail — loudly.
- GET and identity-content paths now distinguish true absence (PDT 404 →
  fame 404) from transport/decode failure (fame 500), and every fetch or
  status-update failure logs loudly at the choke point. The fail-silent
  500 (no journal line) is dead.

## [0.3.10] - 2026-09-25

### Fixed
- Entity list endpoints no longer cap at 100 records (NGHR issue b3209637):
  the tag-search page size is raised to 1000. v0.3.9's fixed window silently
  evicted the oldest records from every list while they stayed GET-able by
  id, and `total` (the returned page count) under-counted — list-based
  consumers could miscount and re-create evicted records.
- This is an interim mitigation, not a complete fix: the window still exists
  in principle (stores past 1000 matching records would truncate again).
  Proper remedy — true-total semantics + cursor pagination — tracked as a
  follow-up.
- Test-code formatting drift fixed (`cargo fmt` clean on the release tree;
  no behavior change).

## [0.3.9] - 2026-09-09

### Added
- `/health` self-reports `{status, service, version}`.
- Crate publish metadata, license files, rewritten README, changelog.

### Security
- Removed internal hostnames from test fixtures and OpenAPI docs (genericized to
  example.com).

## [0.3.8] - 2026-09-07

### Fixed
- Token enrichment failure answers `401` with `WWW-Authenticate` + retryable JSON body
  instead of silently continuing with a role-less principal (which surfaced as lying
  authorization `403`s). Viewer defaults removed from the request path.

## [0.3.7] - 2026-09-02

### Fixed
- Mock-PDT test server persists `auth_context` at create, matching real PDT's wire
  contract — test-green now implies wire-green.

## [0.3.6] - 2026-09-02

### Fixed
- Agent identities are birth-stamped with `auth_context` (team + own `admin_group`) at
  create — the owning agent can read its own charter with zero hand-stamps.

## [0.3.5] - 2026-09-02

### Fixed
- Agent-role `EditAgentIdentity` is resource-scoped: the identity's `admin_group` must be
  in the caller's groups — cross-agent edit deny is now Cedar policy, not an accident.

## [0.3.4] / [0.3.3] / [0.3.2] - 2026-09-01

### Added
- Identity bootstrap-create permitted for agent + service roles (manager agents provision
  reports' charters; tocpi provisions on behalf of the platform).
- Gate guards exercising the real create surface.

## [0.3.1] - 2026-08-31

### Added
- Two-tier test harness: in-process mock PDT (CI default) + real-PDT tier via
  `FAME_IT_PDT_BIN`; self-contained identity suite.

## [0.3.0] - 2026-08-31

### Added
- `PATCH /api/v1/agent-identitys/{id}/content` — governed charter updates with
  fetch-first loud 404, required content (empty = logged wipe), old→new sha256 echo.

## [0.2.0] - 2026-08-23

### Changed
- Workspace-scoped agent identity authorization via stored `admin_group` metadata.

## [0.1.3] - 2026-08-23

### Fixed
- User-role permits for agent-identity CRUD and memory view.

## [0.1.2] - 2026-08-22

### Fixed
- Cedar actually initializes and enforces: empty namespace + attribute-based roles;
  boot fails loudly when `cedar.enabled=true` and init fails (the 0.1.0/0.1.1 fail-open
  bug class — schema never parsed, service booted with zero authorization).
- List/get filter by the entity's full default-tag set, not `type` alone.

## [0.1.1] - 2026-08-12

### Added
- Agent-identity entity — agent charter and self-model as a config-driven entity.

## [0.1.0] - 2026-08-12

### Added
- Initial release: generic TOML-config-driven entity engine over PDT — semantic, episodic,
  and procedural memory per agent, scoped via `X-Instance-Id`, with OIDC auth (pep).

[Unreleased]: https://github.com/Podtan/fame/compare/v0.3.14...HEAD
[0.3.14]: https://github.com/Podtan/fame/compare/v0.3.13...v0.3.14
[0.3.13]: https://github.com/Podtan/fame/compare/v0.3.12...v0.3.13
[0.3.12]: https://github.com/Podtan/fame/compare/v0.3.11...v0.3.12
[0.3.11]: https://github.com/Podtan/fame/compare/v0.3.10...v0.3.11
[0.3.10]: https://github.com/Podtan/fame/compare/v0.3.9...v0.3.10
[0.3.9]: https://github.com/Podtan/fame/compare/v0.3.8...v0.3.9
[0.3.8]: https://github.com/Podtan/fame/compare/v0.3.7...v0.3.8
[0.3.7]: https://github.com/Podtan/fame/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/Podtan/fame/compare/v0.3.5...v0.3.6
[0.3.5]: https://github.com/Podtan/fame/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/Podtan/fame/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/Podtan/fame/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/Podtan/fame/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/Podtan/fame/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/Podtan/fame/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Podtan/fame/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/Podtan/fame/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/Podtan/fame/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/Podtan/fame/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Podtan/fame/releases/tag/v0.1.0

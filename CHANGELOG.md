# Changelog

All notable changes to Fame are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[Unreleased]: https://github.com/Podtan/fame/compare/v0.3.9...HEAD
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

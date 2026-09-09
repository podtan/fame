# Changelog

All notable changes to Fame are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

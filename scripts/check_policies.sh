#!/usr/bin/env bash
# Cedar policy gate for Fame — run before tagging/deploying.
#
# The v0.1.0–0.1.1 fail-open bug: the schema used Podtan:: namespace syntax
# that never parsed, so Cedar init failed and the service booted with zero
# authorization. This script is the mechanical gate that catches that class
# of bug before it ships. `cargo test` also covers it (the enforcement unit
# tests build a CedarAuthorizer from the embedded sources), but the CLI path
# here validates the on-disk files exactly as a release would.
#
# Usage: scripts/check_policies.sh   (from repo root)
# Requires: cedar-policy-cli (cargo install cedar-policy-cli)
set -euo pipefail

cd "$(dirname "$0")/.."

SCHEMA=policies/schema.cedarschema
POLICY=policies/rbac.cedar

echo "── cedar validate ──────────────────────────────────────────"
cedar validate --schema "$SCHEMA" --policies "$POLICY"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# Runtime-faithful entities: principal (+ for workspace controls, the
# resource entity carrying its admin_group attribute — exactly what
# check_permission_scoped builds at runtime).
cat > "$TMP/admin.json" <<'EOF'
[{"uid": {"type": "User", "id": "gate-admin"}, "attrs": {"role": "admin"}, "parents": []}]
EOF
cat > "$TMP/viewer.json" <<'EOF'
[{"uid": {"type": "User", "id": "gate-viewer"}, "attrs": {"role": "viewer"}, "parents": []}]
EOF
cat > "$TMP/user.json" <<'EOF'
[
 {"uid": {"type": "User", "id": "gate-user"}, "attrs": {"role": "user"}, "parents": []},
 {"uid": {"type": "AgentIdentity", "id": "agent-bare"}, "attrs": {}, "parents": []}
]
EOF
WS_GROUP='ws-1a2b3c4d-admins'
cat > "$TMP/ws-member.json" <<EOF
[
 {"uid": {"type": "User", "id": "gate-ws-member"}, "attrs": {"role": "user", "groups": ["${WS_GROUP}"]}, "parents": []},
 {"uid": {"type": "AgentIdentity", "id": "agent-demo"}, "attrs": {"admin_group": "${WS_GROUP}"}, "parents": []}
]
EOF
cat > "$TMP/ws-outsider.json" <<EOF
[
 {"uid": {"type": "User", "id": "gate-ws-outsider"}, "attrs": {"role": "user", "groups": ["ws-somewhere-else-admins"]}, "parents": []},
 {"uid": {"type": "AgentIdentity", "id": "agent-demo"}, "attrs": {"admin_group": "${WS_GROUP}"}, "parents": []}
]
EOF

fail=0

echo "── authorize controls ──────────────────────────────────────"
auth() { # principal-id entity-file action resource expected
  local out
  out=$(cedar authorize --policies "$POLICY" --schema "$SCHEMA" \
    --principal "User::\"$1\"" --action "Action::\"$3\"" \
    --resource "$4" --entities "$2" 2>&1) || true
  if echo "$out" | grep -q "^ALLOW$"; then result=ALLOW; else result=DENY; fi
  if [ "$result" = "$5" ]; then
    echo "PASS  $3 ($1) → $result"
  else
    echo "FAIL  $3 ($1) → $result, expected $5"
    fail=1
  fi
}

# Baseline controls (0.1.2)
auth gate-admin  "$TMP/admin.json"   CreateSemanticMemory 'SemanticMemory::"<_>"' ALLOW
auth gate-viewer "$TMP/viewer.json"  CreateSemanticMemory 'SemanticMemory::"<_>"' DENY
auth gate-viewer "$TMP/viewer.json"  DeleteAgentIdentity 'AgentIdentity::"<_>"'   DENY

# 0.1.3 controls: user — identity + View only; memory writes and deletes denied
auth gate-user   "$TMP/user.json"    CreateAgentIdentity 'AgentIdentity::"agent-bare"' DENY
auth gate-user   "$TMP/user.json"    ViewSemanticMemory 'SemanticMemory::"<_>"'   ALLOW
auth gate-user   "$TMP/user.json"    CreateSemanticMemory 'SemanticMemory::"<_>"' DENY
auth gate-user   "$TMP/user.json"    DeleteAgentIdentity 'AgentIdentity::"<_>"'   DENY

# 0.2.0 controls: workspace-scoped identity create/edit
# (bare resource, no admin_group attr → user's scoped permit cannot match)
auth gate-ws-member   "$TMP/ws-member.json"   EditAgentIdentity 'AgentIdentity::"agent-demo"' ALLOW
auth gate-ws-outsider "$TMP/ws-outsider.json" EditAgentIdentity 'AgentIdentity::"agent-demo"' DENY

exit $fail

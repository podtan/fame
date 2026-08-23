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

# Runtime-faithful entities: ONLY the principal (no Role hierarchy entities,
# no parents) — enforcement.rs never sends anything else.
cat > "$TMP/admin.json" <<'EOF'
[{"uid": {"type": "User", "id": "gate-admin"}, "attrs": {"role": "admin"}, "parents": []}]
EOF
cat > "$TMP/viewer.json" <<'EOF'
[{"uid": {"type": "User", "id": "gate-viewer"}, "attrs": {"role": "viewer"}, "parents": []}]
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

auth gate-admin  "$TMP/admin.json"  CreateSemanticMemory 'SemanticMemory::"<_>"' ALLOW
auth gate-viewer "$TMP/viewer.json" CreateSemanticMemory 'SemanticMemory::"<_>"' DENY
auth gate-viewer "$TMP/viewer.json" DeleteAgentIdentity 'AgentIdentity::"<_>"'   DENY

# 0.1.3: workspace-owner path — user gets identity CRUD + memory View,
# never memory writes or deletes
cat > "$TMP/user.json" <<'EOF'
[{"uid": {"type": "User", "id": "gate-user"}, "attrs": {"role": "user"}, "parents": []}]
EOF
auth gate-user "$TMP/user.json" CreateAgentIdentity 'AgentIdentity::"<_>"'  ALLOW
auth gate-user "$TMP/user.json" ViewSemanticMemory 'SemanticMemory::"<_>"'  ALLOW
auth gate-user "$TMP/user.json" CreateSemanticMemory 'SemanticMemory::"<_>"' DENY
auth gate-user "$TMP/user.json" DeleteAgentIdentity 'AgentIdentity::"<_>"'  DENY

exit $fail

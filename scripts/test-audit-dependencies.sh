#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
cat > "$WORK_DIR/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  tree)
    [[ "$*" == "tree --locked --target all --all-features --prefix none --format {p}" ]]
    printf '%s\n' "$TEST_GRAPH"
    exit "$TEST_TREE_EXIT"
    ;;
  audit)
    printf '%s\n' "$*" > "$TEST_AUDIT_LOG"
    exit "$TEST_AUDIT_EXIT"
    ;;
  *) exit 99 ;;
esac
EOF
chmod +x "$WORK_DIR/cargo"
export PATH="$WORK_DIR:$PATH" TEST_AUDIT_LOG="$WORK_DIR/audit.log"
export TEST_GRAPH='teatro v0.0.0' TEST_TREE_EXIT=0 TEST_AUDIT_EXIT=0
bash "$SCRIPT_DIR/audit-dependencies.sh" --no-fetch
[[ "$(< "$TEST_AUDIT_LOG")" == 'audit --no-fetch --ignore RUSTSEC-2023-0071' ]]
rm "$TEST_AUDIT_LOG"
export TEST_GRAPH=$'teatro v0.0.0\nrsa v0.9.8'
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
test ! -e "$TEST_AUDIT_LOG"
export TEST_GRAPH='teatro v0.0.0' TEST_TREE_EXIT=42
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
test ! -e "$TEST_AUDIT_LOG"
export TEST_TREE_EXIT=0 TEST_AUDIT_EXIT=43
if bash "$SCRIPT_DIR/audit-dependencies.sh"; then exit 1; fi
printf 'Audit guard passed: inactive RSA, active RSA, graph failure, and audit failure.\n'

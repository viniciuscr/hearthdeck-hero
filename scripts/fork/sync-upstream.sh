#!/usr/bin/env bash
# Reconcile upstream/main into fork/main under the change-control model in
# docs/FORK.md. Never pushes: the reconciliation commit is reviewed first.
set -euo pipefail

BASELINE_BRANCH="${BASELINE_BRANCH:-main}"
INTEGRATION_BRANCH="${INTEGRATION_BRANCH:-fork/main}"
UPSTREAM_REMOTE="${UPSTREAM_REMOTE:-upstream}"
UPSTREAM_BRANCH="${UPSTREAM_BRANCH:-main}"
GATE=1
VERIFY_ONLY=0

usage() {
  cat <<'EOF'
Usage: scripts/fork/sync-upstream.sh [--no-gate | --verify]

  --no-gate   Reconcile only; skip the cargo verification gate.
  --verify    Run the verification gate on the current branch and exit.

Environment: BASELINE_BRANCH, INTEGRATION_BRANCH, UPSTREAM_REMOTE,
UPSTREAM_BRANCH.
EOF
}

die() {
  printf 'fork-sync: %s\n' "$*" >&2
  exit 1
}

run_gate() {
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
}

for arg in "$@"; do
  case "$arg" in
    --no-gate) GATE=0 ;;
    --verify) VERIFY_ONLY=1 ;;
    -h | --help) usage; exit 0 ;;
    *) die "unknown argument: $arg" ;;
  esac
done

git rev-parse --is-inside-work-tree >/dev/null 2>&1 || die "not a git work tree"

if [ "$VERIFY_ONLY" -eq 1 ]; then
  run_gate
  exit 0
fi

[ -z "$(git status --porcelain)" ] || die "working tree is dirty; commit or stash first"
git remote get-url "$UPSTREAM_REMOTE" >/dev/null 2>&1 ||
  die "missing remote '$UPSTREAM_REMOTE'; run: git remote add $UPSTREAM_REMOTE https://github.com/RafaelGoulartB/Heronite.git"

git fetch "$UPSTREAM_REMOTE" --tags

git switch "$BASELINE_BRANCH"
git merge --ff-only "$UPSTREAM_REMOTE/$UPSTREAM_BRANCH"

git switch "$INTEGRATION_BRANCH"
if ! git merge --no-ff "$UPSTREAM_REMOTE/$UPSTREAM_BRANCH" \
  -m "merge(upstream): reconcile $UPSTREAM_REMOTE/$UPSTREAM_BRANCH"; then
  cat >&2 <<'EOF'
fork-sync: conflicts above. Resolve them according to docs/FORK.md (upstream
wins outside registered seams), then run:
  git add -A && git commit
  scripts/fork/sync-upstream.sh --verify
EOF
  exit 1
fi

if [ "$GATE" -eq 1 ]; then
  run_gate
fi

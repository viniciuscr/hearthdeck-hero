#!/usr/bin/env bash
# Tags fork/main for a tag-driven release. The tag starts
# .github/workflows/fork-release.yml, which builds the signed Arch repository
# on GitHub Pages and the GitHub release.
#   usage: scripts/fork/release.sh 1.5.0-hearthdeck.1
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

version=${1:?usage: scripts/fork/release.sh <version, e.g. 1.5.0-hearthdeck.1>}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9.-]+)?$ ]] || {
  echo "release.sh: version must look like 1.5.0 or 1.5.0-hearthdeck.1" >&2
  exit 2
}
[[ $(git branch --show-current) == fork/main ]] || {
  echo "release.sh: release only from fork/main" >&2
  exit 2
}
[[ -z $(git status --porcelain) ]] || {
  echo "release.sh: commit or stash the working tree first" >&2
  exit 2
}
git fetch origin fork/main
[[ $(git rev-parse HEAD) == $(git rev-parse origin/fork/main) ]] || {
  echo "release.sh: fork/main is not what origin has; pull or push first" >&2
  exit 2
}

tag="v$version"
git tag -a "$tag" -m "Hearthdeck Hero $tag"
git push origin "$tag"
echo "Pushed $tag; the fork release workflow now builds and publishes it."

# Fork maintenance

Hearthdeck Hero is a fork of [Heronite](https://github.com/RafaelGoulartB/Heronite)
(GPL-3.0-only). This document is the fork's change-control record: how upstream
updates are received, where the fork is allowed to differ, and how every
reconciliation is verified.

The model follows the ASD-TE100 configuration-management discipline: a pristine
baseline, an integration baseline, approved deviations, and a verification gate.

## Baselines

| Branch | Role | Rules |
|---|---|---|
| `main` | pristine upstream baseline | fast-forward only; no fork commits; mirrors `upstream/main` |
| `fork/main` | integration baseline | long-lived; features land here; upstream merged forward; never force-pushed |
| `feat/<slug>` | change request | cut from `fork/main`; squash-merged; deleted after merge |

`upstream` is `https://github.com/RafaelGoulartB/Heronite.git`; `origin` is the
fork. One-time setup:

```bash
git remote add upstream https://github.com/RafaelGoulartB/Heronite.git
git fetch upstream --tags
```

Never merge `fork/main` into `main`, and never push fork commits to `upstream`.

## Change requests

1. Cut the branch from the integration baseline:
   `git switch -c feat/<slug> fork/main`.
2. One logical change per branch. Use Conventional Commits
   (`feat(scope): ...`, `fix(scope): ...`), matching `cliff.toml`.
3. Contain the change (see below) and update the deviation register whenever an
   upstream file is modified.
4. Verify locally: `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`.
5. Squash-merge into `fork/main`.

## Receiving upstream updates

```bash
scripts/fork/sync-upstream.sh            # fetch, fast-forward main, reconcile, verify
scripts/fork/sync-upstream.sh --no-gate  # reconciliation only
scripts/fork/sync-upstream.sh --verify   # verification gate only
```

The script never pushes. It fast-forwards `main`, merges `upstream/main` into
`fork/main` as one reconciliation commit, and runs the verification gate. On a
conflict it stops: resolve according to the deviation register (upstream wins
outside registered seams), commit, then run `--verify`.

Cadence: weekly, or on each upstream release tag. Small, frequent
reconciliations are much cheaper than large ones.

## Verification gate

Identical to upstream CI:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`.github/workflows/fork-ci.yml` runs the gate on pushes and pull requests to
`fork/main`. Upstream `ci.yml` also runs on pull requests, including its Windows
jobs. `fork/main` is never tagged for release with a red gate.

## Change containment

- A new capability lives in a new crate (`crates/<name>`, no GPUI) plus a thin
  adapter in `app/src`. Edit upstream code only at registered seams.
- No repo-wide formatting, renames, moves, or refactors of upstream code.
- Extend through fork-owned traits and adapters; do not edit upstream enums,
  structs, or service signatures when a wrapper can do the job.
- Fork feature switches belong to a fork-owned crate, not to
  `heronite_platform::Feature`.
- Commits that touch upstream files carry a `Fork-Seam: <id>` trailer.
- Keep the deviation surface small and stable: review
  `git diff main...fork/main --name-only` at every reconciliation.

## Branding identity

`crates/fork-brand` is the single source of truth: `Hearthdeck Hero`,
`app.hearthdeck.Hero`, data folder `hearthdeck`, URL scheme `hearthdeck`,
updater `viniciuscr/hearthdeck-hero`. Environment variables keep upstream's
`HERONITE_*` names so upstream tooling and tests keep working.

## Deviation register

Every intentional difference from upstream is recorded here. `Fork-Seam`
trailers and this table must agree. The Movies feature's contracts and
implementation rules live in `docs/MOVIES.md`.

| ID | Upstream configuration item | Deviation | Status |
|---|---|---|---|
| F-001 | `crates/update/src/lib.rs` (`LATEST_API`, `REPO_PAGE`, `RELEASES_PAGE`) | updater reads the fork's release channel | active (asset names and signing key: F-003) |
| F-002 | app identity: `app.heronite.Heronite`, data folder `heronite`, scheme `heronite` | fork identity from `fork-brand`, so both apps coexist | active (runtime; binary and packages: F-003) |
| F-003 | `packaging/*`, `scripts/appimage.sh`, `scripts/sign-release.sh`, binary and package names | fork packages, release asset names and signing key | pending |
| F-004 | `.github/workflows/fork-ci.yml` | fork verification gate (new file, no upstream conflict) | active |
| F-005 | user-facing copy in `app/src`, `crates/i18n` locales, `README.md`, `docs/` | "Heronite" replaced by "Hearthdeck Hero" | pending |
| F-006 | `app/src/collection/{mod,load,nav}.rs`, `app/src/collection/console/sidebar.rs`, `app/src/collection/{actions,game_actions}.rs` | Collection movies space, console space item and Stremio Play action | active |
| F-007 | `crates/data/src/library.rs` | `stremio` source label | active |

### Conflict log

Append one entry per reconciliation that required a manual conflict resolution:
date, upstream commit, files, and resolution.

## Releases

- Tag `fork/main` as `v<upstream>.<fork>` (for example `v1.5.0-fork.1`).
- Version metadata tracks upstream's `Cargo.toml`; the fork suffix is applied at
  packaging time.
- Keep upstream `release.yml`, `flatpak.yml`, and `windows-packages.yml`
  **disabled** in the fork's Actions settings: they use upstream secrets and
  publish to upstream's package channels.
- Until F-003 lands, release assets keep upstream's names (`Heronite-*`) and
  the updater keeps upstream's public key: a fork release signed with another
  key is rejected, which fails closed.
- Add `fork-release.yml` only with the fork's own signing key and channels.
- GPL-3.0-only: keep `LICENSE` and `THIRD-PARTY-NOTICES.md`, state the
  modifications in this file, and publish the source for every distributed
  binary.

## Upstream-first rule

Generic fixes and features are offered upstream on a branch cut from
`upstream/main`; fork-only work stays on `fork/main`. Never mix the two in one
change request. Cherry-pick when a change must live in both.

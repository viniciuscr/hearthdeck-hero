# Releasing Hearthdeck Hero

Releases are **tag-driven** and only come from `fork/main`.

## Cutting a release

Actions → **Fork Release** → *Run workflow*, branch `fork/main`, bump `auto`.
`fork/main` is the repository's default branch, which is what makes the button
appear (GitHub offers it only for workflows that live on the default branch);
the run still refuses any other ref. `scripts/fork/release.py --publish` cuts
the same release from a checkout. Before anything is tagged, the run:

1. refuses any ref but `fork/main`;
2. runs the full gate (`cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace`);
3. works out the next version and its notes from the commits
   (`scripts/fork/release.py`) and prints them;
4. creates the annotated tag `v<version>` — whose message *is* those notes —
   and pushes it.

The same run then builds with Ubuntu 22.04 (the glibc floor), packages with
nfpm 2.47.0, `repo-add`s the Arch package **without signatures** in an
`archlinux:base` container, publishes `arch/x86_64/` to `gh-pages`, and creates
the GitHub release from the tag's notes. It is the same run on purpose: a tag
pushed with `GITHUB_TOKEN` starts no workflow of its own, so waiting for the
tag event would publish nothing.

`main` is the pristine upstream mirror and is never tagged. Feature branches
are never released.

### Seeing what the next release would be

```bash
scripts/fork/release.py            # the version and the notes, nothing tagged
scripts/fork/release.py --bump minor
```

It warns when the tree is dirty, when `HEAD` is not `fork/main`, and when
upstream has released a version `fork/main` has not merged yet.

### Pushing a tag by hand

When the workflow cannot be started from the UI, or to cut the tag from a
checkout, `--publish` does the whole of it — the version, the notes, the tag
and the push:

```bash
scripts/fork/release.py --publish
```

It refuses a dirty tree, a branch other than `fork/main`, and a head that
differs from `origin/fork/main`. A failed build of a release that already has
its tag is recovered with *Re-run failed jobs* on that workflow run, not with a
new tag.

By hand it is four commands, and the cleanup flag matters:

```bash
tag=$(scripts/fork/release.py --tag)
scripts/fork/release.py --notes > /tmp/notes.md
git tag -a --cleanup=whitespace "$tag" -F /tmp/notes.md
git push origin "$tag"
```

`--cleanup=whitespace` is not optional: git's default (`strip`) deletes every
line that starts with `#`, which takes the notes' markdown headings with it
(and needs git 2.43 or newer).

## Version numbers

- Git tag: `v1.7.0-hearthdeck.1.5.0`
- The base (`1.7.0`) is the tree's `[workspace.package] version`, which the
  sync brings in from upstream; the script reads it and never guesses it.
- The counter (`1.5.0`) is the fork's own semver: `feat` → minor, `fix` and
  everything else → patch, `!` or `BREAKING CHANGE` → major. It keeps counting
  across upstream bases (`v1.6.1-hearthdeck.1.2.0` was followed by
  `v1.7.0-hearthdeck.1.2.0`, then `.1.3.0`); the base alone decides which
  release is newer.
- CI sets the build's version to the whole tag, so the updater compares
  `1.7.0-hearthdeck.x` against `1.7.0-hearthdeck.y` — an app on an older
  release of the same base can update itself.
- Arch `pkgver`: `1.7.0.hearthdeck.1.5.0` (pacman does not accept `-` in
  `pkgver`).

## The release notes

`scripts/fork/release.py` writes them from the commits, in two halves:

- **What we changed** — the fork's own commits (`--first-parent`, so the
  upstream side of each sync merge is left out), grouped by Conventional Commit
  type. Documentation, chores, CI and tests are dropped.
- **From upstream** — the commits the sync merges brought in, summarised: the
  top scopes, one sentence for the rest, and a link to upstream's own release
  notes. Listing sixty upstream commits would bury the fork's work.

They live in the annotated tag and nowhere else, so `git show <tag>` is the
record of what a release shipped, and re-running a failed build publishes the
same text.

## One-time setup

### Default branch

`fork/main` is the default branch, and that is what makes the *Run workflow*
button appear: GitHub offers it only for workflows that live on the default
branch. `fork-release.yml` must therefore stay on `fork/main` and must not be
added to `main`, which is the pristine upstream mirror (`sync-upstream.sh`
fast-forwards it, so an extra commit there would stop the fast-forward).

### GitHub Pages

Settings → Pages → Build and deployment:

- Source: **Deploy from a branch**
- Branch: **gh-pages** / **/ (root)**

The workflow creates `gh-pages` on the first release.

### Unsigned Arch repository

No signing key is required. The workflow runs `repo-add` without `--sign`;
packages and the database are unsigned. This is the simple path for the TV.

### TV machine

Add to `/etc/pacman.conf`:

```ini
[hearthdeck]
SigLevel = Optional TrustAll
Server = https://viniciuscr.github.io/hearthdeck-hero/arch/$arch
```

Then:

```bash
sudo pacman -Syyu
sudo pacman -S hearthdeck-hero
```

`TrustAll` is acceptable for a private TV repository. Add signatures later
without changing the package name or URL.

## Local test package

```bash
cargo build --release --locked -p heronite -p heronite-epic-login
scripts/packages.sh "1.7.0.hearthdeck.1.5.0"
sudo pacman -U --needed dist/hearthdeck-hero-*-x86_64.pkg.tar.zst
```

`nfpm` is required; the CI uses 2.47.0.

## The TMDB application key

TMDB authenticates every call with an *application* key: there is no anonymous
login, and no per-user key. The release build therefore carries the fork's own,
compiled in through `HEARTHDECK_TMDB_API_KEY` (`option_env!` in
`crates/watch-tmdb`); `.github/workflows/fork-release.yml` passes the repository
secret `TMDB_API_KEY` into the build. A tag built without the secret is not
fatal — TMDB features report that the build carries no key instead of offering a
Connect that cannot work.

- Getting it: themoviedb.org → Settings → API → request a Developer key (one per
  application, free, instant).
- Normal path: `scripts/bootstrap.sh` asks for the key and writes it to
  `~/.config/hearthdeck/tmdb-api-key` (0600); the app adopts that file into its
  credential store the first time Settings opens, so a machine can be re-keyed
  without a rebuild.
- Compiling it in stays possible: `export HEARTHDECK_TMDB_API_KEY=...` before
  `cargo build` (that is what the release secret does).
- Never commit the key. It is world-readable once the app ships, which is normal
  for a client application and is why the secret lives in CI, not in the tree.

## The Simkl client id

Simkl works the same way, with a **client id** instead of a key: it identifies
the application, and the person only ever approves a code from the TV (the
device / PIN flow of AUTH V2). The release carries the fork's own through
`HEARTHDECK_SIMKL_CLIENT_ID` (`option_env!` in `crates/watch-simkl`), fed by the
repository secret `SIMKL_CLIENT_ID`.

- Getting it: simkl.com → your account → Apps → Developers → create an app of
  type **TV, devices & command line** (AUTH V2 — a V1 app is refused by the
  device endpoint). `client_id` is public and safe to ship; only the user tokens
  are secrets.
- Normal path: `scripts/bootstrap.sh` asks for it and writes it to
  `~/.config/hearthdeck/simkl-client-id` (0600); the app adopts that file into
  its credential store the first time Settings opens.
- A tag built without the secret is not fatal: Settings → Watch → Accounts says
  the client id is missing instead of offering a Connect that cannot work.

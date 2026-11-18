# Releasing Hearthdeck Hero

Releases are **tag-driven** and only come from `fork/main`.

## Tagging

```bash
# after the fork gate is green and fork/main matches origin
scripts/fork/release.sh 1.5.0-hearthdeck.1
```

The script refuses a dirty tree, a branch other than `fork/main`, or a head
that differs from `origin/fork/main`. It creates `v<version>` and pushes it.

The tag starts `.github/workflows/fork-release.yml`:

1. verify the tag commit is on `origin/fork/main` and run the full gate
   (restoring `main`'s Rust cache; a tag never saves one, because caches are
   scoped to the ref and no later run could read it);
2. build with Ubuntu 22.04 (the glibc floor), package with nfpm 2.47.0;
3. `repo-add` the Arch package **without signatures** in an `archlinux:base` container;
4. publish `arch/x86_64/` to the `gh-pages` branch;
5. create the GitHub release.

`main` is the pristine upstream mirror and is never tagged. Feature branches
are never released.

## Version numbers

- Git tag: `v1.5.0-hearthdeck.1`
- Arch `pkgver`: `1.5.0.hearthdeck.1` (pacman does not accept `-` in `pkgver`)
- Rebuild of the same source: increment the last number, e.g. `.2`

## One-time setup

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
scripts/packages.sh "1.5.0.hearthdeck.1"
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

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

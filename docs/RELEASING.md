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

1. verify the tag commit is on `origin/fork/main` and run the full gate;
2. build with Ubuntu 22.04 (the glibc floor), package with nfpm 2.47.0;
3. `repo-add --sign` the Arch package in an `archlinux:base` container;
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

### Signing key

Create a dedicated signing key (no passphrase for CI, or keep it offline and
sign locally), export it, and add repository secrets:

```bash
gpg --quick-generate-key "Hearthdeck Hero packages <packages@hearthdeck.invalid>" rsa4096 sign 5y
gpg --list-secret-keys --keyid-format LONG
gpg --armor --export-secret-keys <KEY_ID>   # GPG_PRIVATE_KEY
```

Add `GPG_PRIVATE_KEY` (ASCII-armored secret key) and `GPG_KEY_ID` (the long
key id) to the repository's Actions secrets.

`repo-add --sign` signs the database. The generated `.sig` files are published
next to the database.

### TV machine

Import the public key once:

```bash
gpg --armor --export <KEY_ID> | sudo pacman-key --add -
sudo pacman-key --lsign-key <KEY_ID>
```

Add to `/etc/pacman.conf`:

```ini
[hearthdeck]
SigLevel = Required DatabaseOptional
Server = https://viniciuscr.github.io/hearthdeck-hero/arch/$arch
```

Then:

```bash
sudo pacman -Syyu
sudo pacman -S hearthdeck-hero
```

For a private test before signing is configured, `SigLevel = Optional TrustAll`
works, but it is not the released setup.

## Local test package

```bash
cargo build --release --locked -p heronite -p heronite-epic-login
scripts/packages.sh "1.5.0.hearthdeck.1"
sudo pacman -U --needed dist/hearthdeck-hero-*-x86_64.pkg.tar.zst
```

`nfpm` is required; the CI uses 2.47.0.

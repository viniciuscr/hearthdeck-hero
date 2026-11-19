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

`.github/workflows/fork-ci.yml` runs the gate (Linux and Windows) on pushes to
`fork/main`, and is the only ref that writes Rust caches. Upstream `ci.yml`
runs the same gate on pull requests and restores those entries, because a pull
request can read its base branch's caches. Making `fork/main` the repository's
default branch would extend that to every other ref — a tag, which can read
only the default branch, included. `fork/main` is never tagged for release
with a red gate.

## Change containment

- A new capability lives in a new crate (`crates/<name>`, no GPUI) plus a thin
  adapter in `app/src`. Edit upstream code only at registered seams.
- Fork-owned code is not registered: `crates/watch*`, `app/src/fork/**`,
  `app/src/collection/watch_playback.rs` and `docs/WATCH*.md` are additions, not
  deviations. The register lists only upstream files a fork commit edits.
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
implementation rules live in `docs/WATCH.md`.

| ID | Upstream configuration item | Deviation | Status |
|---|---|---|---|
| F-001 | `crates/update/src/lib.rs` (`LATEST_API`, `REPO_PAGE`, `RELEASES_PAGE`) | updater reads the fork's release channel | active (asset names and signing key: F-003) |
| F-002 | app identity: `app.heronite.Heronite`, data folder `heronite`, scheme `heronite` | fork identity from `fork-brand`, so both apps coexist | active (runtime; binary and packages: F-003) |
| F-003 | `packaging/*`, `scripts/packages.sh`, `scripts/appimage.sh`, `scripts/install.sh`, `crates/update`, binary and package names | fork Linux/Arch packages, binaries, desktop identity and release channel | active (Linux/Arch; Flatpak/Windows pending) |
| F-004 | `.github/workflows/fork-ci.yml` | fork verification gate (new file, no upstream conflict) | active |
| F-005 | user-facing copy in `app/src`, `crates/i18n` locales, `README.md`, `docs/` | "Heronite" replaced by "Hearthdeck Hero" (app and i18n done; README/docs pending; `console_controls.json` in all four locales still carries upstream's two strings) | active |
| F-006 | `app/src/collection/{mod,load,nav}.rs`, `app/src/collection/console/sidebar.rs`, `app/src/collection/{actions,game_actions}.rs`, `app/src/pages/settings/console.rs` (one dispatch line) | Collection `Space::Watch`, console space item, the Watch Play action and Settings → Watch's own sections | active |
| F-007 | `crates/data/src/library.rs` | `stremio` source label | active |
| F-008 | `app/src/console/{themes,theme_rows}.rs`, `app/src/console/theme_rows/host.rs`, `app/src/collection/console/{sources,layouts/grid,filters}.rs`, `app/src/pages/settings/{section,mod,console}.rs`, `app/src/pages/settings/controls/mod.rs`, `app/src/collection/mod.rs` | Watch theme preset, Watch settings registration, per-space prefs, editor lock, source/grid-filter helpers, filter panel and one-time migration of game-shaped Watch files; the controller's options are global, so they are read from and written to the Games prefs whatever space is on screen | active |
| F-009 | `.github/workflows/{release,flatpak,windows-packages}.yml` | upstream release workflows guarded to the upstream repository | active |
| F-010 | `app/src/collection/{actions,game_actions,render_focus,grid_drawer}.rs`, `app/src/collection/focus/mod.rs`, `app/src/collection/console/{mod.rs,game/mod.rs,layouts/spotlight/mod.rs}` | Native Watch playback: the stream list and Play action on both detail surfaces | active |
| F-011 | `docs/ARCHITECTURE.md` | the watch crate map (the fork-owned watch crates are absent upstream) | active |
| F-012 | `app/src/collection/console/{mod.rs,game/mod.rs,game/hero.rs}`, `app/src/collection/hltb_ui.rs` | a fork-owned film page for Watch titles (`console/watch_page.rs`) and no HowLongToBeat lookups for them | active |
| F-013 | `crates/launch/src/ffmpeg_install.rs` (one test) | `only_the_programs_asked_for_are_downloaded` accepts a host that already has ffmpeg on the `PATH` — the Linux case the module documents — instead of expecting the "not downloaded on" error there | active |
| F-014 | `app/src/collection/store_media/mod.rs` (`sync_store_media`) | no store media and no Steam lookup by name for a Watch title: a film has no store page, and its name would otherwise be searched on Steam as a game title | active |
| F-015 | `crates/data/src/console_theme/mod.rs` (`MAX_TABS`), `app/src/console/theme_rows/tests.rs` (the tab-limit test and the theme-take assertions) | at most **six** tabs (LB / RB) instead of four: the Watch space's home, movies, series and shelf fill four on their own and a theme brings its own, so four left no room to add one | active |
| F-018 | `.github/workflows/fork-release.yml` (the build step's `env`) | the release build carries the fork's application credentials — TMDB's API key from the `TMDB_API_KEY` secret (`HEARTHDECK_TMDB_API_KEY`) and Simkl's client id from `SIMKL_CLIENT_ID` (`HEARTHDECK_SIMKL_CLIENT_ID`) — because neither provider has an anonymous login and both authenticate the *application* | active |
| F-017 | `crates/backup/src/catalog.rs` (one rule in `classify_config`) | Watch's provider credentials (`<provider>-auth.json`) are `Part::Logins` instead of `Settings`: a backup may exclude them, and the cloud sync never uploads logins. Watch's history, shelves and settings stay `Settings`, so a restore keeps them | active |
| F-016 | `app/src/console/theme_rows/{,commands,host}.rs`, `app/src/console/themes.rs`, `app/src/collection/console/{mod,sources}.rs`, `app/src/collection/console/layouts/{cinema,carousel,dashboard,grid,xbox,spotlight}`, `app/src/collection/console/{hero.rs,game/{mod,hero,sheet}.rs}`, `app/src/pages/settings/{console_mode/{editors.rs,preview/layouts.rs},controls/mod.rs}`, `crates/i18n/locales/*` | themes are **presented per space**: `space_theme::validate` runs on every load and change (removes only what a space cannot draw) and `present` runs when a theme is taken (adds the tabs and rails Watch needs, never renames). Watch edits its own tabs, shelves and hero — the sources, shelf kinds and hero lines are the space's — and the desktop editor only ever writes the Games prefs (`File: app/src/console/theme_rows/host.rs`, `ThemeState::new`) | active |
| F-019 | `app/src/collection/{vm,data,card,list_row}.rs`, `app/src/collection/console/tile.rs` | a Watch placeholder is dimmed **whatever the theme's "grey uninstalled" says**: one `CardVm.imported` field and its assignment, a helper `data.rs` duplicates from `app/src/fork/watch/sources.rs`, and three one-line grey conditions (`card.rs`, `list_row.rs`, `console/tile.rs`). The duplicate helper goes in P5; `docs/WATCH-TITLES.md` owns the transition and its upstream budget | active |

F-019 is the Watch titles transition's only new deviation so far. F-020 (one
branch in `app/src/collection/copies.rs`, so "Play with \<provider\>" reaches
the Watch picker for a copies group) is reserved by `docs/WATCH-TITLES.md` §2
and is registered when that phase lands, i.e. when a second stream provider
ships.

### Conflict log

Append one entry per reconciliation that required a manual conflict resolution:
date, upstream commit, files, and resolution.

- **2026-11-14, upstream `bffbe5bc`** (27 commits; the French locale and the
  "Back to game" overlay work among them). Three files and three locale files
  conflicted, all where the F-005 rename met new upstream copy:
  - `app/src/collection/focus_left.rs`: upstream's "Back to game" arrow beside
    Play, kept for games and disabled in the Watch space (which has no install
    or launch of its own).
  - `app/src/overlay/panel/rows.rs`: upstream's new "Back to game" row and its
    `game_away` condition, with the fork's "Back to Hearthdeck Hero" row.
  - `app/src/overlay/window/windows.rs`: upstream's notice/foreground fix, with
    the fork's window titles.
  - `crates/i18n/locales/{de,es,pt-BR}/overlay.json`: upstream's new keys, plus
    the renamed "Back to Hearthdeck Hero" key.
  - `crates/i18n/locales/fr/**`: French is new upstream, and the fork's English
    strings are renamed. Every French key whose English text carried "Heronite"
    was renamed with its text ("Hearthdeck Hero"), and `fr/watch.json` is new
    for the Watch strings.

- **2026-11-15, upstream `46441a4f`** (74 commits, v1.7.0: store media and
  trailers, the quick launcher, the Controller settings page, Steam links,
  MangoHud, console sounds). Ten files conflicted, again where the F-005 rename
  and the fork's spaces met new upstream code:
  - `app/src/collection/focus/mod.rs`: upstream's new Store media and Links
    sections kept, and both hidden in the Watch space along with screenshots,
    timeline, related and achievements — a film page carries the streams
    instead (F-010).
  - `app/src/console/themes.rs`: the fork's per-space prefs kept. Upstream's
    controller-option installation is grafted onto the *Games* prefs, which own
    the global controller options; a Watch update never writes them (F-008).
  - `app/src/pages/settings/section.rs`: upstream's Controller and Quick
    launcher sections merged into the App group, beside the fork's Watch group.
    `GROUPS` stays 7 and `ALL` grows 24 → 26.
  - `app/src/system/hub/launch.rs`: upstream's `exit_when_idle` branch (a quick
    launcher instance stays, so it does not wait for the notice) with the
    fork's notification title (F-005).
  - `crates/i18n/locales/fr/{console_settings,dialogs,epic,overlay,settings_pages,stores_pages}.json`:
    French is the fork's rebranded copy, so upstream's file was taken whole and
    the rename re-applied rather than merged hunk by hunk. Upstream now ships
    French for `quick_launch` too (new file, rebranded).
  - Rebranded outside the conflicts, in Rust strings and locales together
    (the coverage test matches them): the 12 keys upstream added in
    `epic.json`, `quick_launch.json` and the Stores → Steam copy, plus
    `de/es/pt-BR/quick_launch.json` and the new `epic.json` and
    `stores_pages.json` keys.
  - One new cross-file collision, resolved towards upstream because
    `watch.json` is fork-owned: upstream's `dialogs.json` now translates
    "Unlink" and "Not linked", so `watch.json` in de/fr/pt-BR takes upstream's
    text (de "Verknüpfung entfernen", fr "Non associé", pt-BR "Desvincular" and
    "Não vinculado").

- **2026-11-16, upstream `7a5feddf`** (16 commits, including the fork's
  Ludusavi emulator-manifest fix merged as `19181c55` with two review
  follow-ups). Four locale files conflicted, again where the F-005 rename met
  new upstream copy:
  - `crates/i18n/locales/{de,es,fr,pt-BR}/quick_launch.json`: upstream's file
    was taken whole and the rename re-applied, adding "Hearthdeck Hero could
    not read the library, so no games are listed." and "{key} needs Ctrl, Alt
    or {logo} with it; only the F keys, Print Screen and Pause work alone or
    with just Shift.".
  - Rebranded outside the conflicts (F-005), in the code the locales must
    match: `app/src/quick_launch/view.rs` (the new "could not read the
    library" string) and `app/src/quick_launch/window.rs` (the window title is
    `fork_brand::NAME`).

## Releases

- Tag `fork/main` as `v<upstream>-hearthdeck.<n>` (for example
  `v1.5.0-hearthdeck.1`); never tag `main`.
- Releases are tag-driven: the tag starts `.github/workflows/fork-release.yml`,
  which gates the commit, builds `hearthdeck-hero` packages and publishes an
  unsigned Arch repository to `gh-pages`.
- `scripts/fork/release.sh` creates and pushes the tag. See
  `docs/RELEASING.md` for the one-time Pages setup.
- Arch `pkgver` replaces the tag's hyphen with a dot
  (`1.5.0.hearthdeck.1`); increment the last number for a rebuild.
- Keep upstream `release.yml`, `flatpak.yml`, and `windows-packages.yml`
  **disabled** in the fork's Actions settings: they use upstream secrets and
  publish to upstream's package channels.
- Fork Linux packages are `hearthdeck-hero`; the AppImage is
  `Hearthdeck-Hero-x86_64.AppImage`. The updater accepts both the new and old
  asset names during the transition.
- GPL-3.0-only: keep `LICENSE` and `THIRD-PARTY-NOTICES.md`, state the
  modifications in this file, and publish the source for every distributed
  binary.

## Upstream-first rule

Generic fixes and features are offered upstream on a branch cut from
`upstream/main`; fork-only work stays on `fork/main`. Never mix the two in one
change request. Cherry-pick when a change must live in both.

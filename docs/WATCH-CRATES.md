# Watch crates — package layout and provider contract

Status: accepted plan. `docs/WATCH.md` owns the feature's behaviour; this
document owns how it is packaged so more than one provider can exist.

## Why more than one crate

Watch began as one crate for one provider. It now holds three layers at once:
the Stremio addon protocol, the Stremio provider (account, datastore, Cinemeta,
the local service) and the app-facing playback and Collection projection.
Adding YouTube, OpenTV or anything later inside it repeats every seam.

Two things stay provider-neutral and must be reusable: the addon protocol (those
providers are Stremio-compatible addons) and the mpv player. One thing must stay
independent of every provider: the local state and the Collection projection.

## Crates

```
app
└── heronite-watch              composition root: registry, space availability,
    │                           browse → Collection, playback orchestration
    ├── heronite-watch-stremio  the Stremio provider
    │   └── heronite-watch-addon
    │       └── heronite-watch-core
    ├── heronite-watch-state    local files + Collection projection
    ├── heronite-watch-player   mpv
    └── heronite-watch-core     domain types and ports
```

| Crate | Responsibility | May depend on | Must not |
|---|---|---|---|
| `heronite-watch` | provider registry, `available()`, browse → synthetic `Library`, the single post-session progress write, the app's import surface | `heronite-watch-{core,state,player,stremio}`, `heronite-data` | GPUI, provider protocol code, file formats of its own |
| `heronite-watch-core` | `WatchItem`, `StreamChoice`, `WatchProgress`, `PosterShape`, `AuthKey`, `WatchProvider`, `Ctx`, `Transport`, `Credentials`, `Throttle`, `finished` | serde, serde_json, anyhow, url, chrono, ureq | `heronite-*`, files, processes, provider names |
| `heronite-watch-addon` | Stremio addon protocol: manifest, catalog, meta, stream, `AddonClient`, size and seeders parsing | `heronite-watch-core` | account, datastore, endpoints, Cinemeta constants |
| `heronite-watch-player` | mpv discovery, argument construction, IPC supervision, `PlaybackOutcome`, stream-origin policy | `heronite-data` (`proc`), url, serde_json, anyhow | protocol or provider types |
| `heronite-watch-state` | `store` (movies, per-provider credentials) and `collection` (the Heroic-format projection) | `heronite-watch-core`, `heronite-data` | network, providers, GPUI |
| `heronite-watch-stremio` | endpoints, link code, `addonCollectionGet`, datastore and merge, Cinemeta catalogs, streaming server, `impl WatchProvider` | `heronite-watch-{core,addon}` | files, processes, `heronite-data` |

A provider crate takes everything it needs from `Ctx`; it never opens a file or
spawns a process. That is what keeps a provider testable on fixtures alone.

CR-1 keeps the present type names (`MovieItem`, `MovieProgress`) so the move
changes no code; they become `WatchItem` and `WatchProgress` with the provider
contract in CR-2.

## Contracts

```rust
pub trait WatchProvider: Send + Sync {
    fn id(&self) -> &'static str;                  // "stremio"; also GameInfo.runner/source
    fn catalogs(&self) -> Vec<CatalogDescriptor>;  // the rails it contributes, in order
    fn browse(&self, ctx: &Ctx<'_>, catalog: &CatalogDescriptor) -> Result<Vec<WatchItem>>;
    fn streams(&self, ctx: &Ctx<'_>, item: &WatchItem) -> Result<Vec<StreamChoice>>;
    fn resolve(&self, ctx: &Ctx<'_>, stream: &StreamChoice) -> Result<ResolvedStream>;
    fn read_progress(&self, _: &Ctx<'_>, _: &str) -> Result<Option<WatchProgress>> { Ok(None) }
    fn write_progress(&self, _: &Ctx<'_>, _: &WatchProgress) -> Result<()> { Ok(()) }
}

pub struct Ctx<'a> { pub http: &'a dyn Transport, pub credentials: &'a dyn Credentials }

/// Opaque to core: each provider decodes its own credential type.
pub trait Credentials: Send + Sync {
    fn get(&self, provider: &str) -> Result<Option<Secret>>;
    fn set(&self, provider: &str, secret: &Secret) -> Result<()>;
    fn clear(&self, provider: &str) -> Result<()>;
}

pub struct ResolvedStream { pub url: Url, pub origin: StreamOrigin }

pub enum StreamOrigin { LocalService, Remote }
```

- `browse` is asked per catalog, not once per provider: a provider's rails differ
  in kind and order, and a live provider (OpenTV) has no catalogues at all.
- `read_progress` exists beside `write_progress` because a provider that keeps
  progress remotely must be able to resume from it; a live provider defaults both.
- `resolve` returns the final URL and its origin. Torrent work (the local Stremio
  service) stays inside the Stremio crate; the player accepts `Remote` only over
  https and http only for `LocalService` on loopback.
- Credentials are per provider and opaque, so YouTube's OAuth tokens and a
  provider's endpoint list do not have to become `AuthKey`.

### App-facing surface

The app imports these paths and no others; the facade keeps them stable. The
integration test also uses the root re-exports `AuthKey` and `Transport`.

| Path | Items |
|---|---|
| `heronite_watch` | `available`, `Endpoints`, `UreqTransport`, `AuthKey`, `Transport`, `Result` |
| `heronite_watch::account` | `create_link`, `read_link`, `LinkOffer`, `LinkPoll` |
| `heronite_watch::store` | `read_auth`, `write_auth`, `clear_auth` |
| `heronite_watch::collection` | `ROOT`, `library` |
| `heronite_watch::playback` | `streams`, `play`, `PlaybackRequest` |
| `heronite_watch::player` | `PlaybackOutcome` |
| `heronite_watch::types` | `StreamChoice` and the item types the UI shows |

The facade re-exports items, never whole crates: a `pub use … as module` would
publish `MpvSession`, `DatastoreClient` and the file stores as app API, and a
module and a re-export cannot share a name (`collection` is the case).

## Data ownership

- `heronite-data` is upstream and depend-only. Watch adds no module, key,
  `SOURCE_IDS` entry, `Feature` variant or table to it. It uses `json_store`,
  `paths`, `temps`, `proc` and `Library`.
- `heronite-watch-state` owns `<config>/movies.json` and
  `<config>/<provider>-auth.json`. The credential file is `0600`; its directory
  is created as today, because it is shared configuration. A 0700 secret-only
  directory is a separate hardening change.
- `heronite-watch-state::collection` owns `<cache>/watch-library/**`, written
  through `DataPaths` accessors and read with `Library::load_with`. It is
  regenerated on every load and never merged with the game library.
- There is no watch data crate: it would duplicate atomic and secret-safe writes
  that `heronite-data` already owns (`docs/WATCH.md` rule 10).
- An upstream change to those primitives fails the fork's gate at compile time,
  which is the intended failure mode.

## Provider routing

The provider name is the `GameInfo.runner`/`source` value the projection already
writes. Playback takes it from the game: `PlaybackRequest.provider` and the
provider argument of `streams`. No screen learns a provider's name.

## Invariants the refactor preserves

These are behaviour, not style; the move must keep all of them.

1. No write throttle is introduced. `play` writes once locally and once remotely
   after mpv exits. `Throttle` stays a pure predicate with no production caller.
2. The feed is built continue-watching first, then the provider's top catalogs,
   deduplicated by kind and id keeping the first copy, so a title in both rails
   is not marked top. An unlinked account shows the catalogues only.
3. The projection's JSON stays byte-identical, including its deliberate key
   asymmetry (raw id for the local and timestamp maps, `<provider>:<id>` for
   metadata and art), the explicit `null`s and the insertion order.
4. `finished` has one definition. Error messages, including
   "Playback is only available on Linux.", keep their text.
5. Progress keys stay raw ids until the provider that needs namespacing lands.

## Change requests

One branch per change request, cut from `fork/main`, squash-merged. The gate is
`scripts/fork/sync-upstream.sh --verify` plus a pull request into `fork/main`,
which runs `fork-ci.yml` and upstream `ci.yml` (Windows included). CR-2 adds the
two missing CI matrix entries, `--all-features` and `--no-default-features`, once
providers sit behind features: the default build alone never proves the façade's
wiring.

| CR | Type | Content |
|---|---|---|
| CR-1 | `refactor(watch)` | pure moves: crate skeletons, then core + state, addon, player, projection, Stremio provider. No public API change. |
| CR-2 | `feat(watch)` | provider registry, provider routing in `streams`/`PlaybackRequest`, removal of dead code, curated facade. |
| CR-3 | `feat(watch)` | prerequisites for provider #2: provider-scoped projection keys, `(provider, id)` progress keys with a read fallback, typed credentials, per-catalog rails, stream-origin policy. |

Dead code is removed in its own commit: `deep_link` (no caller since the
Stremio-app handoff was dropped), `progress::{Throttle, record}` (no production
caller) and `MoviesFile::{addons, cinemeta}` (never read; removing them changes
the bytes of the next `movies.json` write and that delta is recorded).

### Phases of CR-1

| Phase | Move | Acceptance |
|---|---|---|
| R0 | six manifests and empty crate roots; `[lints] workspace = true`, `workspace.package` fields | gate |
| R1 | `core` (types, auth, http, throttle, finished) and `state::store`; its callers move in the same phase | files and bytes unchanged; store tests |
| R2 | `addon`; the integration test's fixtures stay where they are | `tests/protocol.rs` passes |
| R3 | `player`; all types target-independent, only discovery and start are `cfg(unix)` | argument tests; Windows job |
| R4 | `state::collection`; the five `json!` builders move literally | golden-file test |
| R5 | `stremio`; the facade composes providers, store and player | fake-transport tests |

## Adding a provider

1. A crate `crates/watch-<provider>` implementing `WatchProvider`; it receives
   `Ctx` and never touches files or processes.
2. Its credential type and save path, registered in the facade behind a cargo
   feature (`default = ["stremio"]`).
3. `read_progress`/`write_progress`, or the defaults for a provider without
   history.
4. Its catalogs, and the item fields the screens filter on.
5. A `provider:id` projection key and the progress migration (CR-3) before the
   provider ships.
6. Fixtures and a fake transport; no test reaches the network.

`docs/WATCH.md` and the crate map in `docs/ARCHITECTURE.md` change in the same
request, and the deviation register gains only upstream files.

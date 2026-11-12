# Watch (Stremio first) — implementation contract

Status: specification for Phases 0–5. `docs/FORK.md` owns the fork's
change-control rules; this document owns the Watch feature's definitions and
the rules that apply while building it.

> **Revision 2026-11-12 — native playback only, films first.**
> The Stremio desktop app is no longer required and is no longer used for
> playback. Play resolves the film's streams from the `stream` addons installed
> on the linked account (Torrentio and friends), asks the installed Stremio
> Service to fetch the chosen torrent, and hands its HTTP stream to `mpv`. The
> desktop Focus panel and the console game page both list the streams inline;
> picking one starts playback. Series, seasons and episodes remain out of scope
> for this revision.

> **Revision 2026-11-11 — the Watch space is the Collection.**
> The separate page was removed. `crates/watch::collection` writes Stremio
> films and series into a synthetic Heroic-format root, and `CollectionView`
> loads it under `Space::Watch`. The same grid, Focus panel, shelves, filters,
> game page and console layout render it; only the `Library` behind them
> changes. Console mode is the primary target; the desktop menu carries the
> same switch. The 2026-11-12 revision replaces the old Play handoff.

> **Revision 2026-11-11 — series and providers.**
> Watch covers films and series. Stremio is the first provider: it fills the
> same `Library` through `crates/watch`. Future providers (YouTube, HBO Max,
> Prime Video, Netflix) are adapters that produce the same records; they must
> not require changes to the Collection or console screens. The provider name
> travels in `GameInfo.runner` / `source` and in `WatchItem.provider`.

> **Revision 2026-11-11 — Watch settings and migration.**
> Settings → Watch owns the Stremio account and the home rails; console
> Customize edits the same Watch theme. A game-shaped `console-watch.json` or
> `watch-ui-state.json` is backed up as `*.json.bak` and replaced once with the
> Watch defaults: Home / Movies / Series, and Continue watching / Top movies /
> Top series. The Watch theme is fork-owned (`app/src/console/watch_theme.rs`),
> not a transformed game preset.

## Scope

**In:** Linux/CachyOS, films and series, a Stremio account linked by code,
discovery through Cinemeta, streams from the account's `stream` addons (Torrentio
is the expected one), playback through the installed Stremio Service and a
supervised `mpv`, and watch-progress read/write through Stremio's datastore.

**Out (v1):** episode selection inside a series, Windows playback, an in-app
video renderer, any plugin/addon host, and any change to the game catalog, its
data files or its launch paths.

**Non-negotiable:** a gamepad must control every playback path we ship, either
directly or through a documented handoff. No secret may reach a log, an error
string or a crash report. No write may delete or un-watch an account record.

## Boundaries

| Unit | Responsibility | May depend on | Must not |
|---|---|---|---|
| `crates/watch` | Stremio account, datastore, addon protocol, streaming-server client, mpv supervisor, local watch store | `heronite-data`, `heronite-images`, `heronite-platform`, `heronite-launch`, `ureq` | GPUI, `app`, or any upstream crate edited in place |
| `app/src/collection/` (`Space::Watch`) | The same Collection/console screens, loading the proxy `Library` | `crates/watch`, `heronite-data` | A second grid, Focus panel or shelves implementation |
| `app/src/collection/nav.rs`, `collection/console/*`, `collection/palette/*` | Navigation and shell route entries | — | Feature logic |

**Composition rule.** New behavior lives in the new crate and is imported by
the app. Extend upstream types with extension traits; wrap them in newtypes
only when every call site is under our control. Do **not** patch an upstream
crate, add inherent methods to its types, or assume a wrapper intercepts calls
made by code that still names the original type.

**Editable upstream files.** Only the closed enums and exhaustive matches that
cannot be extended from outside: the console `Screen` route in
`app/src/collection/console/mod.rs`, the sidebar `Item::parse` in
`app/src/collection/console/sidebar.rs`, and the Collection header/menu rows.
Each edit is one arm, carries `Fork-Seam: F-006`, and is listed in
`docs/FORK.md`.

**Forbidden shortcut.** Do not add a `Feature::Watch` variant. Route the
space's availability through `heronite_watch::available()`; the `Feature` enum
is an upstream closed enum and would widen the deviation surface for no
benefit.

## Definitions

These are the types Phase 1 creates, with the wire shapes they map to.

```rust
/// Injected everywhere; tests never reach a real host.
pub struct Endpoints {
    pub api: Url,                 // https://api.strem.io
    pub link: Url,                // https://link.stremio.com
    pub cinemeta: Url,            // https://v3-cinemeta.strem.io
    pub streaming_server: Url,    // http://127.0.0.1:11470
}

/// The session key. Debug/Display redact it; it is never serialized.
pub struct AuthKey(String);

/// The Stremio library record, as stored in datastore `libraryItem`.
pub struct LibraryItem {
    pub id: String,               // JSON `_id`, e.g. "tt26657236"
    pub name: String,
    pub kind: String,             // JSON `type`: "movie", "series", "channel"
    pub poster: Option<Url>,
    pub poster_shape: PosterShape,
    pub removed: bool,
    pub temp: bool,
    pub ctime: Option<DateTime<Utc>>,   // JSON `_ctime`
    pub mtime: DateTime<Utc>,           // JSON `_mtime`
    pub state: LibraryItemState,
    #[serde(flatten)] pub extra: serde_json::Map<String, Value>,
}

/// Milliseconds, as Stremio stores them.
pub struct LibraryItemState {
    pub last_watched: Option<DateTime<Utc>>,
    pub time_watched: u64,
    pub time_offset: u64,
    pub overall_time_watched: u64,
    pub times_watched: u32,
    pub flagged_watched: u32,
    pub duration: u64,
    pub video_id: Option<String>,       // `tt<id>` or `tt<id>:<season>:<episode>`
    pub watched: Option<String>,        // opaque base64 bitfield
    pub last_vid_released: Option<DateTime<Utc>>,
    pub no_notif: bool,
    #[serde(flatten)] pub extra: serde_json::Map<String, Value>,
}

/// A catalog/meta item from Cinemeta or an installed addon.
pub struct MovieItem {
    pub id: String,
    pub kind: String,                   // "movie" in v1
    pub name: String,
    pub year: Option<String>,           // may be "2021–2025"
    pub poster: Option<Url>,
    pub background: Option<Url>,
    pub logo: Option<Url>,
    pub description: Option<String>,
    pub runtime: Option<String>,
    pub genres: Vec<String>,
    pub addon: String,                  // addon id, for provenance
}

/// One entry of a `stream` response, normalized.
pub struct StreamChoice {
    pub addon: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub info_hash: Option<String>,      // torrent
    pub file_idx: Option<u16>,          // torrent file index
    pub url: Option<Url>,               // direct/debrid
    pub behavior_hints: Value,
    pub size_bytes: Option<u64>,        // parsed from the description when present
    pub seeders: Option<u32>,
}

/// Local, non-secret state (`movies.json`).
pub struct MovieProgress {
    pub id: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub updated_at: DateTime<Utc>,
    pub finished: bool,
}
```

**Secret storage.** `stremio-auth.json` is written with
`heronite_data::temps::write_atomic(path, bytes, Some(0o600))`, in a directory
created `0700`. `json_store::write` uses the process umask and is not used for
this file. `crates/platform/src/secret.rs` is Windows DPAPI only and is not
used on Linux.

## Contracts

### 1. Account link (verified locally)

- `GET {link}/api/create` → `{success, code, link, qrcode, result, error}`.
- `GET {link}/api/read?code=XXXX` → pending and expired both answer an error;
  a linked code answers `{result:{authKey, success:true}}`.
- A code is single-use and short-lived; request a fresh code per attempt.
- `authKey` is opaque (about 44 characters). No user object and no refresh
  token are returned; re-linking is the only recovery.
- Never log the key, the code, or a full link URL.

### 2. Datastore (read verified; write **PROBE REQUIRED**)

Envelope, source `stremio-core`:

```json
{"authKey":"…","collection":"libraryItem","ids":[],"all":true}   // datastoreGet
{"authKey":"…","collection":"libraryItem"}                        // datastoreMeta
{"authKey":"…","collection":"libraryItem","changes":[LibraryItem]} // datastorePut
```

- Read path: `datastoreGet` with `all:true`; `datastoreMeta` is the cheap change
  detector (`[id, epoch_ms]` pairs).
- Continue Watching predicate: `(!removed || temp) && time_offset > 0`.
- `timeOffset`/`duration` are milliseconds; season/episode come from
  `video_id`, never from `state.season`/`state.episode`.
- Write path: read → merge into the **full** item → send the full item. Preserve
  `removed`, `temp`, `_ctime`, unknown keys and untouched state fields. Update
  only `state.time_offset`, `state.duration`, `state.last_watched`,
  `state.video_id` and `_mtime`.
- Before the first write: a no-op probe (send the current item unchanged) to
  prove the envelope; keep a local backup of the item.
- Writes are throttled (≥30 s) and idempotent; on failure keep local progress
  and retry later. Never write while offline; never delete a record.

### 3. Addons (request shape verified locally)

- `GET {transport}/manifest.json`.
- `GET {transport}/catalog/{type}/{id}.json?…` for catalogs.
- `GET {transport}/meta/{type}/{id}.json` for details.
- `GET {transport}/stream/{type}/{id}.json` for streams; a missing `streams`
  list is an error, an empty array is a valid "nothing to play".
- Cinemeta is the catalog/meta source and declares no `stream` resource.
- Torrentio is the stream source; **PROBE REQUIRED** for its actual fields
  (`infoHash`/`fileIdx` vs `url`, `behaviorHints`, size/seeders text).
- Cap response bodies (1 MiB), set timeouts, and name addons by host in errors;
  a transport URL may carry the user's debrid credentials in its path.

### 4. Local streaming server (**PROBE REQUIRED**)

- Base URL: `http://127.0.0.1:11470` (the installed Stremio service).
- `GET /settings` → `{baseUrl, values}`; `baseUrl` is what clients stream from.
- `POST /{infoHash}/create` with
  `{"torrent":{"infoHash":"…"},"peerSearch":{"sources":["dht:…", …],"min":40,"max":200}}`.
- Stream URL: `{baseUrl}/{infoHash}/{fileIdx}` (confirm the exact shape).
- A direct `url` stream bypasses the service.
- Health: probe `/settings` before playback. When the service is down, the play
  fails with that reason: there is no Stremio-app handoff. Never assume
  "localhost" is trusted: the service is a separate process, not our code.

### 4a. Stream resolution

- Read the session key, then `addonCollectionGet`; keep the addons whose
  manifest serves `stream` and whose `types` include the title's kind.
- Ask each addon once (`stream/{type}/{id}.json`), name it by manifest id, and
  keep only entries with a `url` or an `infoHash`.
- Order by seeders, then size; the first entry is what Play starts when the
  person has not picked another.
- An empty list is a valid answer. No stream addon at all is an error that
  names Torrentio and says where to install it.

### 5. Playback

- **Native only:** `mpv --fullscreen --input-ipc-server=<socket> --start=<seconds> -- <url>`.
  Use `--` before the URL and a scheme allowlist (`http`, `https`); reject a URL
  that begins with `-`. IPC socket `0600` in a `0700` directory, owned by the
  app, unlinked on exit.
- **Supervision:** own mpv through `heronite_data::proc` (`guard`/`Run`/cancel)
  on a dedicated thread; poll `time-pos`, `duration`, `pause`, `eof`; hold
  `KeepAwake`; return focus to the app on exit. A shared `AtomicBool` stops the
  session: mpv receives `quit`, and is killed if it does not exit within five
  seconds.
- **Gamepad modes, probed at runtime:** (a) mpv `--input-gamepad=yes` with a
  shipped `input.conf`; (b) evdev→IPC bridge only when mpv is focused and the
  overlay does not hold a grab; (c) shell fallback where the app stays in front
  and reads the pad itself. Never run two readers at once.
- **Failure:** one explicit error with the reason (no service, no mpv, no
  peers). The person can pick another stream from the same list; the app never
  silently opens another application.

### 6. Progress

- Position is milliseconds; resume uses the stored `position_ms` when it is
  older than the finished threshold and less than the duration.
- Mark finished at Stremio's 70 % threshold (`WATCHED_THRESHOLD_COEF`) but do not
  set `flagged_watched`/`times_watched` unless the user marks it watched.
- Local `movies.json` is the source of truth between writes; Stremio is the
  cross-device source.

## Flows

- **Link:** `Unlinked → Pending(code) → Linked`; expired/consumed code returns to
  `Unlinked` with a "get a new code" action.
- **Browse:** `Idle → Loading → Ready | Empty | Error`; a failed addon refresh
  keeps the last good rows and marks them stale.
- **Play:** `Picked → Resolving streams → Picked stream → Service create → Native → Playing → Returned → Persist`;
  a failure at any step goes to `Failed(reason)` while the stream list stays on
  screen for another pick.
- **Controller:** `Local → HandedOff → Returning → Local`; `OverlayGrabbed`
  blocks the shell reader. Suspend the console reader before spawning a player
  and re-arm it on window activation.
- **Write:** `Clean → Pending → Writing → Written | Failed(hold)`; a retry is the
  same item, never a partial update.

## Rules

1. **Compose, don't patch.** New behavior in `crates/watch`; no edits to
   upstream crates; only the registered `app` seams.
2. **One arm per seam.** Every upstream enum match gains exactly one arm;
   `Fork-Seam: F-006` on the commit; update the register in the same change.
3. **No new `Feature` variants.** Availability comes from
   `heronite_watch::available()`.
4. **Blocking off the UI.** Library crates are blocking; short calls use
   `cx.background_spawn`, long-lived work gets a dedicated thread. No I/O in
   `render`.
5. **Secrets stay secret.** 0600 file, `0700` directory, redacted `Debug`,
   no key/code/URL in logs, errors or crash reports.
6. **Treat third parties as untrusted.** Validate schemes, cap bodies, set
   timeouts, redact URL paths, and never pass an addon string as an mpv option.
7. **Atomic, preserving writes.** Read → merge → write; unknown fields
   preserved; no deletions; backup before the first account write.
8. **Idempotent progress.** No-op probe, throttle, retry, offline hold.
9. **One controller owner.** The console reader stays suspended while mpv
   runs; re-arm it when mpv exits; never let two readers consume the same pad.
10. **Reuse the house pipeline.** App image loader, `i18n` keys, `console::kit`,
    `PadTarget`, and the fork-owned Watch QR renderer
    (`fork::watch::settings::qr`); no parallel implementations.
11. **Locales in the same change request.** Every new `tr!` key lands in
    pt-BR, es and de or the i18n coverage test fails.
12. **Cross-platform compilation.** Protocol/domain code compiles everywhere;
    unix sockets and child processes live under `#[cfg(unix)]` with a typed
    `Unsupported` fallback. Windows CI must stay green.
13. **Tests never touch the network.** Fixtures + a local `TcpListener` stub;
    real-account tests are `#[ignore]` and opt-in.
14. **Every phase ends green.** `cargo fmt --all -- --check`,
    `cargo clippy --workspace --all-targets -- -D warnings`,
    `cargo test --workspace`; each phase is self-contained, no `todo!()`.
15. **Document probes.** When a probe confirms or contradicts a contract, update
    this document in the same change.

## Planned deviation

| ID | Files | Deviation | Status |
|---|---|---|---|
| F-006 | `app/src/collection/{mod,load,nav}.rs`, `app/src/collection/console/sidebar.rs`, `app/src/collection/{actions,game_actions}.rs` | Collection `Space::Watch`, console space item, Watch Play action | active |
| F-010 | `crates/watch/src/{playback,player,types,lib}.rs`, `app/src/collection/watch_playback.rs`, `app/src/collection/{actions,game_actions,render_focus,grid_drawer}.rs`, `app/src/collection/focus/mod.rs`, `app/src/collection/console/{mod.rs,game/mod.rs,layouts/spotlight/mod.rs}` | Native Watch playback: account addon streams, Stremio Service, mpv, inline stream lists on both detail surfaces | active |

## Phase 0 probe checklist

1. `GET http://127.0.0.1:11470/settings` with the Stremio service running;
   record `baseUrl` and the exact envelope.
2. `POST http://127.0.0.1:11470/<infoHash>/create` with a known movie torrent;
   record the accepted body and the resulting `baseUrl/<infoHash>/<fileIdx>`
   playback URL.
3. Play that URL in mpv; record resume/IPC behavior.
4. The account's `addonCollectionGet`; confirm Torrentio appears with the
   `stream` resource, then `stream/movie/<tt-id>.json` for its real fields.
5. Torrentio `stream/movie/<tt-id>.json`; record the real field names.
6. `datastorePut` no-op on a test account; record the response and confirm the
   item is unchanged.
7. `mpv --version`; confirm `--input-gamepad` support on CachyOS.

## Phase exit criteria

- **P0:** every probe above recorded in this file; unknowns have a decision.
- **P1:** `crates/watch` compiles on Linux and Windows; fixture tests pass; no UI.
- **P2:** link + datastore + merge/write with mock-server tests; real-account
  test ignored.
- **P3:** `Space::Watch` in the Collection (desktop), proxy Library, locales; gate green.
- **P4:** console Watch item, same shelves and game page, inline stream rows.
- **P5:** native mpv through the Stremio Service, gamepad modes; hardware
  checklist green.

## Probe log

- **2026-11-10, CachyOS (the TV machine).** `mpv` and Stremio are not
  installed; `/opt`, `PATH` and the Flatpak list have neither. Phase 0 probes
  are blocked until they are installed. Phase 1 proceeded because every test
  uses fixtures and a fake transport, never the network. Re-run the probe
  checklist above after installing the Stremio desktop app (or Stremio
  Service) and `mpv`.
- **2026-11-10, Phase 5.** Native playback is implemented in
  `crates/watch/src/player.rs`: a guarded mpv process group, a private `0700`
  IPC directory, `--` before the URL, and `--input-gamepad=yes` only when
  `mpv --list-options` declares it. Streams with an `infoHash` go through the
  local Stremio service; direct URLs go straight to mpv. Progress is saved
  locally and written back on exit. The hardware checklist (probe 7 and an
  end-to-end playback) remains open because neither program is installed on
  this machine.
- **2026-11-12, native playback wired.** `crates/watch/src/playback.rs`
  resolves films through the linked account's `stream` addons and starts
  `mpv` from the service's HTTP stream; `app/src/collection/watch_playback.rs`
  shows the list and owns the session. The Stremio-app handoff is gone from
  the Play action. Tests use fixtures and a fake transport; the service, mpv
  and real Torrentio fields still need the probe checklist on the TV machine.



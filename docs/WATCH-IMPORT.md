# Watch → import: pulling a provider's watch state into the local library

Status: **draft for review** (2026-11-18). The rules are code now:
[`WATCH-SYNC-RULES.md`](WATCH-SYNC-RULES.md) is the pseudo-code spec, and
`crates/watch-core/src/sync_rules.rs` implements its tables with the tests.
This document is the reasoning behind them.

## 1. Why this document exists

Watch currently moves data **one way**: a playback or a manual mark is written
locally, then queued in an outbox and delivered to each enabled tracker
(`docs/WATCH-TRACKERS.md`). What the person does on their phone, on the provider's
site, or in another app never comes back.

This document proposes the other direction — an **import** — and asks reviewers to
attack it. The hard questions are not "how do we call the API": they are identity,
conflicts, echo suppression, multi-provider composition, and how much machinery is
justified.

## 2. Vocabulary (these words are load-bearing)

| Term | Meaning | Crate |
|---|---|---|
| **Media provider** | owns titles, accounts, streams (`stremio`) | `heronite-watch-stremio` |
| **Catalog source** | what may be browsed (`cinemeta`, `tmdb`) | `heronite-watch-catalogs` |
| **Metadata source** | what describes a title (`tmdb`, `cinemeta`, `simkl`) | `heronite-watch-metadata` |
| **Tracker** | where *our* watching is pushed (`simkl`) | `heronite-watch-simkl` |
| **Remote state** | whatever a provider holds about the person's watching | — |

"Datastore" is **Stremio's** name for its own remote state (`datastoreGet` /
`datastorePut` on the `libraryItem` collection: `state.timeOffset`, `temp`,
`removed`, `_mtime`). It is not a general concept and must not leak outside that
crate. Every provider spells its remote state differently:

- **Stremio**: datastore `libraryItem` records with a per-item `_mtime`.
- **Simkl**: `/sync/activities` (buckets) + `/sync/all-items`, `/sync/playback`,
  `/sync/ratings`; watchlist statuses `watching`, `plantowatch`, `hold`,
  `completed`, `dropped`; `removed_from_list` timestamps.

**No provider is the main one.** Everything is off until chosen, any combination
(including none) is valid, and no code path may assume a particular provider
answers. The **local files are the hub**; providers are peers.

## 2.5 The sync contract is ours, never a provider's

The core contains no provider's word. Two concepts happen to look like Simkl's
because it came first; the contract is what the code sees, and each provider
describes itself in these terms:

| Our term | Meaning | Simkl says | Trakt says |
|---|---|---|---|
| **change query** | "did anything move since this snapshot?" | `/sync/activities` | `/sync/last_activities` |
| **snapshot** | opaque bytes we store verbatim; the core never parses them | the whole activities JSON | the whole last_activities JSON |
| **history** | watched facts (title/episode, when) | `/sync/all-items`, `/sync/history` | `/sync/history` — plays with their own ids |
| **playback** | resume position | `/sync/playback` (a `progress` percent) | `/sync/playback` (a `progress` percent) |
| **lists** | membership of a kind (`watchlist`, `favourites`, `custom`) | `plantowatch`, `/lists/favorites`, `/lists/{id}` | `/sync/watchlist`, `favorited_at`, `/users/{id}/lists` |
| **removal detection** | how we learn something left a list | stamps (`removed_from_list`) and an id diff | **id diff only** — its change query says *when*, not *what* |
| **request extras** | what a call needs beyond auth | query params `client_id`/`app-name`/`app-version` | headers `trakt-api-version`, `trakt-api-key` |
| **collection / ownership** | what a person *has*, not what they plan to watch | no equivalent | `/sync/collection` — **not a list kind** |

```rust
trait WatchSync {
    fn capabilities(&self) -> SyncCapabilities;          // history, playback, lists per kind, ratings
    fn snapshot(&self) -> Result<Vec<u8>>;               // opaque: stored, never read by the core
    fn changed(&self, since: &[u8]) -> Result<Changed>;  // which axes moved
    fn pull(&self, since: Option<&[u8]>, axes: Changed) -> Result<Pulled>;
}
```

`Changed` and `Pulled` are **ours** — `Pulled { histories, positions, memberships,
removals }` in our own types. A provider that cannot answer an axis says so in
`capabilities()`, and the merge never asks for it.

## 3. What exists today

Local files (`~/.config/hearthdeck/`, mode 0600, atomic writes via `json_store`):

| File | Holds | Lifetime |
|---|---|---|
| `movies.json` | `MovieProgress` per `provider:id`: `position_ms`, `duration_ms`, `updated_at`, `finished`, `video_id`, `imdb`, `episodes{}` of `EpisodeProgress` (`finished`, `season`, `number`) | truth: what playback resumes from |
| `watch-personal.json` | `PersonalLibrary`: `watchlist`, `favourites` (full `ShelfTitle` snapshots), `lists` (reserved) | user data |
| `watch-sessions.json` | `WatchSession` per playback run | append-only log; may be deleted |
| `watch-tracking.json` | outbox: `events[]`, `notices[]`, per-tracker `Failure` (attempts, `next_at`), `MAX_EVENTS = 500` | disposable once delivered |
| `watch-settings.json` | `catalog_sources`, `metadata_sources`, `home_rails`, `stream_providers`, `trackers` | settings |
| `watch-details/` (cache) | `TitleDetails` per title, `SCHEMA = 3`, 7-day freshness | disposable |

Mode 0600 is the **credential** files' business (`store.rs`); the JSON stores go
through `json_store`, which leaves the umask alone.

Push path (`crates/watch/src/trackers.rs`): `record_playback`, `record_manual`
append to the outbox; `drain` folds per title/episode (newest wins), delivers one
event at a time to each enabled tracker, with 1/5/30-minute backoff and `Unmatched`
as permanent. Playback never touches the network on its own thread.

Reads today: `heronite_watch::progress::local(app, provider, id)` and the rails
(`crates/watch/src/library.rs`): provider rails (Stremio's own catalog id is
`continue-watching`; `continue:<provider>` is the *settings* id that selects it),
catalog rails, and a `personal` rail carrying watchlist+favourites. Note the
wording: `tracking::queue` folds a new event into the newest state per title, and
`drain` only delivers.

## 4. Goals

1. A person who watched, or marked something watched, **elsewhere** sees it here:
   in the watched marks, in the Continue watching rail, and (later) in the
   watchlist.
2. Nothing that happens on this machine is lost or duplicated because of an import.
3. With every provider off, the app behaves exactly as it does today.
4. Adding a second provider is additive — no migration, no merge order, no rewrite
   of the first.
5. The sync is **automatic and periodic while the app runs** — no button to
   remember — and its cost is bounded and inspectable: a small cursor per provider,
   one change-query call per tick, a handful of requests when something moved.

## 5. Non-goals (explicit)

- No background service and no polling while the app is closed: the tick runs
  only in-process, only for providers that are connected, and backs off when
  nothing changes. **Lists are bidirectional by decision** (§7.4); history is not
  (§9).
- No remote *mirror* on disk: we do not keep the provider's items.
- No ratings, memos, custom lists, or rewatches (Simkl: PRO-gated) in this plan.
- No rail-level merging: rails read the local state, which is where the merge
  already happened.
- No import of `hold`/`dropped` into our states as if they were local truth.

## 6. Provider notes: Simkl (verified against the cached docs)

These are facts about *one* provider, kept here so the model above stays neutral.


Source: `https://api.simkl.org/guides/sync`, `.../resources/rate-limits`,
`/api-reference/redirect`, `/conventions/list-statuses`, `/guides/scrobble`.

**Two-phase model**

- Phase 1 (once per account): read `GET /sync/activities` and keep the **whole**
  response in memory; pull the full library; **only then** save the snapshot, and
  not at all if any pull failed. Reading activities *after* the pull loses changes
  made during it.
- Phase 2 (every poll): `GET /sync/activities`; if top-level `all` is unchanged,
  stop (one request). Otherwise fetch only the buckets that moved, passing
  `date_from=<saved top-level all>` **exactly as returned**.
- **Identity needs a flag.** `ids.imdb` is *guaranteed* only with
  `extended=ids_only` (or `full`). The bare `/sync/all-items` example in the
  cached OpenAPI does carry `imdb`, but the anime defaults carry `mal`/`anidb`/
  `anilist` instead — so every pull that feeds identity asks for `ids_only` and
  never trusts the default.
- Episode-level state: `extended=full&episode_watched_at=yes` on the delta; the
  first sync adds `include_all_episodes=yes` for a full baseline, and **any delta
  that can touch `completed`/`dropped` needs it too** — those statuses skip
  episode loading by default.
- `include_all_episodes=yes` **synthesizes** rows sharing the show's last-watched
  time; `=original` returns only recorded rows with real dates. Imported rows are
  approximate and are never shown as an exact watch time.
- `watched_at` near `1970-01-01` is the "I don't remember" placeholder: treat it as
  watched with no date, sorted last, never displayed literally.
- `/sync/playback` returns only **open** sessions, with `progress` (0–100) and no
  duration: the position comes from the local duration, and a session that is no
  longer in the list must clear the imported position. Two caveats the first
  draft missed: `progress` **rounds to an integer on read** (a position is
  accurate to ~1 % of runtime, never exact), and sessions **expire** by plan
  (Free 7 days, PRO 30, VIP 90) with no API notification — an imported position is
  a hint, never a bookmark we own.
- Oversized pulls answer `400 max_items`; split per type or per status.
- Deletions: when `removed_from_list` moved, pull `extended=simkl_ids_only` and
  diff against our saved id set.

**Auth & limits**

- Token required for `/sync/*`; the token-free surface is records by Simkl id,
  episode lists, `/redirect`, trending and calendar files.
- 10 GET/s and 1 POST/s, counted per user on AUTH V2; per-user daily quota
  (Free 500 / PRO 1,000 / VIP 10,000). Exempt endpoints are free on both.
- `400 {"error":"RATE_LIMIT"}` is the 20-second per-user **write** lock (matching
  the name case-insensitively — the docs print both spellings), not a quota.
- Three different 429s: bare `rate_limit` is the per-second burst — retry in about
  a second and **ignore `Retry-After`, which carries the daily reset**;
  `user_limit_exceeded` is the daily quota — wait `Retry-After`;
  `app_limit_exceeded` is V1-only.
- Oversized responses: `400 max_items` (20,000 items / 60,000 with
  `simkl_ids_only`, plus "Too many episodes") splits per type → per status →
  `include_all_episodes=original`.
- Access tokens live 7 days: refresh before a pull; `401` → sign in again (no retry
  loop), `412 client_id_failed` → stop and wait, `403 oauth2_token_required` → a
  V1 token on a V2 endpoint.
- Apps with an older `client_id` still see `notinteresting` where `dropped` now
  appears; normalize it at parse time.
- Do not poll without user interaction.

### 6a. Episodes

A show is never one fact. The pull asks `extended=full&episode_watched_at=yes`, and
the first pull also `include_all_episodes=yes`, so every watched episode arrives
with its season, number and stamp; a later delta asks only for what moved.
`episode_watched_at=yes` shares the show's last-watched stamp across its episodes —
honest for a one-time baseline, which is why the flag is not `original` there.

Applying an episode matches an existing record by season and number (playback keys
episodes by the provider's id) and finishes it, with provenance. A title marked
"not watched" here is never re-finished by a pull.

### 6c. A library too big

`400 {"error":"max_items"}` means "narrow the request", and the ladder is by media
type, then by status. The pull reads the code from the refused body and walks down
it, merging what each answer brings; a 400 that is not about size is still an
error and does not start the ladder.

## 6b. Provider notes: Trakt (verified against the cached blueprint)

A different shape, not a translation of Simkl's — which is the point of §2.5:

| Our concept | Trakt's own thing |
|---|---|
| change query | `GET /sync/last_activities` — OAuth-required; `all` plus per-type buckets `watched_at`, `collected_at`, `rated_at`, `watchlisted_at`, `favorited_at`, `paused_at`, `dropped_at`, and `lists.updated_at` |
| history | `/sync/history/{type}` — **plays**, each with a 64-bit `id`, `watched_at` and `action` (`scrobble`/`checkin`/`watch`); removed by item **or by play id** |
| playback | `/sync/playback/{type}` with `progress`; one entry deletable by `/sync/playback/{id}`; `/scrobble/stop` saves 1-79 % as a pause |
| watchlist | `/sync/watchlist` add/remove — membership only, no status vocabulary |
| favourites | a `favorited_at` bucket exists; the write path must be **read from the blueprint before use**, never guessed |
| **collection** | `/sync/collection` + `/remove` — what the person *owns*; **not** a list kind (see the matrix) |
| custom lists | `/users/{id}/lists`, `/lists/{id}/items/{type}/{sort_by}/{sort_how}` — paginated |
| ratings | `/sync/ratings/{type}/{rating}` add/remove — a later slice |
| removals | nothing in the change query says *what* was removed: a removal needs a full-list diff, exactly like Simkl's `simkl_ids_only` |
| pagination | `page`/`limit` with `X-Pagination-*` response headers |

What this costs the neutral model: **removal detection is a declared mode**
(`stamps` / `diff` / `none`), not an assumption; **ownership is not a list kind**;
and the transport must carry **per-provider headers** as well as per-provider query
params — Simkl puts its app identity in the URL, Trakt in headers.

## 7. Data model proposed

### 7.1 Local stays the hub, unchanged in shape

`movies.json`, `watch-personal.json`, `watch-sessions.json` remain the truth of
"what happened here", extended with **provenance** so imports can be distinguished
and undone:

```jsonc
// MovieProgress gains (all with serde(default), so old files read), in the
// file's own casing — MovieProgress is snake_case, unlike most Watch types:
"origin": "local" | "imported",       // who last wrote the *watched* fact
"imported_from": "simkl",             // which provider, when imported
"imported_at": "2026-11-18T10:00:00Z",
"remote_updated_at": null             // the provider's own stamp (see §9)
```

Do **not** add `#[serde(rename_all)]` to `MovieProgress`: `position_ms` and
`duration_ms` have no `default`, so a rename would make every existing
`movies.json` fail to parse and silently empty the library. Per-episode records
get the same fields — but `EpisodeProgress` *is* camelCase, so they are spelled
`importedFrom`/`importedAt` there. An episode can be imported while the title's
position stays local, and vice versa.

`watch-personal.json` entries gain the same trio, plus the **tri-state** needed for
the watchlist (see 7.4).

### 7.2 One cursor per provider — no mirror

```
config/watch-import/simkl.json
config/watch-import/stremio.json
```

```jsonc
{
  // Opaque and provider-owned: Simkl's activities JSON, Trakt's last_activities,
  // Stremio's per-item mtimes. The core stores these bytes and never reads inside.
  "snapshot": { /* … */ },
  "lastPullAt": "2026-11-18T10:00:00Z",
  // Only for providers whose removal mode is "diff": the id set each list had at
  // snapshot time, by list kind. Empty for a provider that stamps removals.
  "listIds": { "watchlist": [472214, …], "favourites": [] },
  "failure": { "attempts": 1, "nextAt": "…", "error": "…" }
}
```

- `snapshot` + `lastPullAt`: incremental pulls; the snapshot's *meaning* is the
  provider's business (§2.5).
- `listIds`: the membership **last seen** in each list kind (§7.4). Mandatory for
  every provider, `diff` or `stamps`: it is the only way to tell "removed there"
  from "never was there". Advanced only after a tick fully applies.
- the push watermark is **not** here: it lives on the outbox as a bounded
  `pushed` log (§7.5), because `prune` would erase it from either side of a pull.
- No items, no titles, no vocabulary. Deleting this file costs one full pull.

The file's **path and atomic write belong to `heronite-watch-state`** (a new
`watch-state/import.rs`), because `heronite-watch-simkl` has no `heronite-data`
dependency and cannot resolve `AppPaths` (§12a). The provider crate owns only the
typed payload it parses; `heronite-watch` consumes the normalized result.

### 7.3 Identity

Remote items are keyed by external ids; local records by `provider:id`.

```
canonical(title) = imdb (tt…) → provider:id
```

`MovieProgress.imdb` already stores the IMDb id; `ShelfTitle.id` is the provider's
id (IMDb for Stremio/Cinemeta). There is **no local tmdb id**, so a tmdb tier
would be dead code — review finding. Import maps provider ids through `ids.imdb`
(which requires `extended=ids_only`, §6) and **counts items with no IMDb id** as a
skip, surfaced in the import report; they cannot be matched with confidence.

Alternative considered: fuzzy title+year matching. Rejected: an import that
silently attaches a watch to the wrong film is worse than one that misses it.

### 7.4 Lists: the local list is *the* list, providers are inputs and outputs

Nothing here is Simkl-specific. Every provider declares whether it **has a
watchlist**, and if so whether we may **read** it, **write** it, or both (§7.6):
Stremio's library, Simkl's `plantowatch`, Trakt's watchlist are the same role. Any
combination is valid — one, several or none.

The list the person sees is **the local one**, always. Per title it is:

- **unset** — nobody decided here yet; a remote `in` may fill it.
- **in** — in the list; sent to every provider we may write to.
- **out** — removed here on purpose; the removal is sent, and no import re-adds it.

**Lists mirror in both directions — but "not in the remote list" is not the same
as "removed there".** That distinction is the whole rule, and getting it wrong
would delete a title the person had just added here. With only two inputs (local
state, current remote state) the two cases are indistinguishable, so every list
keeps a **third**: the membership we last saw (`cursor.listIds`, §7.2).

Reconciliation is a three-way merge per title:

| Local | Last seen remotely | Remote now | Result |
|---|---|---|---|
| `in` | absent | absent | **never was there → push the add** (never remove locally) |
| `in` | `in` | absent | **removed remotely → remove locally**, notice "Simkl removed 1 title" |
| `in` | `in` | `in` | nothing |
| `out` | `in` | `in` | **removed locally → push the removal** |
| `out` | `in` | absent | both agree: already gone |
| `out` | absent | `in` | **added remotely → add locally** |
| `in`/`out` | different from both | both changed since the last sync | newest change wins; on an exact tie the local state wins |

So: add a film here while it is **not** in the provider's list → it goes *to* the
provider; remove it on the phone → a later tick sees "seen there, gone now" and
removes it here. Absence alone never deletes anything.

Two consequences worth stating: the last-seen set is therefore **mandatory** for
every provider (a `stamps` provider adds an instant, it does not replace the set),
and it is only advanced **after** a tick fully applied — otherwise a half-applied
tick would move the baseline and lose the difference.

**Two tracker accounts plus an existing local list, concretely.** Local holds
*A, B*; Simkl holds *B, C*; Trakt holds *A, D*:

1. First sync → local becomes *A, B, C, D* (union of the three), and the next
   local change sends each title to the side that lacks it.
2. The person removes *A* here → *A* leaves local and is removed from both
   providers.
3. The person removes *C* in Simkl's app → the next tick sees Simkl's list no
   longer has *C*, removes it from local, says "Simkl removed 1 title", and sends
   the removal to Trakt (which still had it). All three agree again.

The cost of step 3 is real and accepted: a removal on a phone deletes on the TV.
History is different — a watched mark is **never** subtracted by a provider (§9),
because a lost watch cannot be recovered while a lost list entry is one tap.

**Three kinds of list, not one.** The rules above are per kind, and each provider
declares a capability **per kind** — never one "lists" switch:

| Kind | Local shape | What we know of the providers |
|---|---|---|
| watchlist | `PersonalLibrary.watchlist` | Simkl `plantowatch` (free); Stremio's library (saved titles); Trakt's watchlist |
| favourites | `PersonalLibrary.favourites` | Simkl `/lists/favorites` (+`/remove`) — **beta, PRO/VIP**, `403 premium_only` on a free account; Trakt, AniList and MAL have their own |
| custom | `PersonalLibrary.lists` (already reserved in the file) | Simkl `/lists`, `/lists/{id}/items` (+`/remove`) — **beta, PRO/VIP**; Trakt user lists; AniList/MAL |

So the earlier claim "no provider has favourites" was wrong — most providers do.
The capability is what varies: Simkl's watchlist is free, its favourites and custom
lists are a paid, beta surface. A switch for a kind the account cannot serve stays
**visible and refused with its reason** ("Simkl favourites need Pro"), because the
answer is a plan, not a bug.

**Custom lists are bound, not guessed.** A provider's lists carry names and ids we
do not own; matching by name would silently merge a local "Anime" with a provider's
"anime" — or worse, two unrelated lists with the same word. So a local list is
linked to **at most one** provider list, explicitly, in Settings (kind `custom`,
provider, list id); an unlinked list stays local. Name matching is never a
fallback.

The schema change is **not** "a field on the entry": `watchlist` is a
`Vec<ShelfTitle>` and removing an entry deletes it, so `out` would have nowhere to
live. It is a separate small ledger — `listState: { "<provider>:<kind>:<id>":
{ state: "in" | "out", origin, imported_at } }` — migrated from the current vec
(absent in the ledger = unset). It is the only change that touches the film page's
reads.

### 7.6 What a provider can do decides which switches exist

| Capability | Stremio | Cinemeta | TMDB | Simkl |
|---|---|---|---|---|
| streams | yes | — | — | — |
| catalog (browse, search) | — | yes | yes | yes (S4) |
| metadata | — | yes | yes | yes |
| history write | yes (datastore) | — | — | yes |
| history read | yes | — | — | yes |
| playback read | yes | — | — | yes |
| watchlist read/write | its library (not wired yet) | — | — | yes (`plantowatch`, free) |
| favourites read/write | — | — | — | yes (beta, PRO/VIP) |
| custom lists read/write | — | — | — | yes (beta, PRO/VIP) |

| Capability | Trakt |
|---|---|
| streams | — |
| catalog | — (lists are browsed, not a catalogue of titles) |
| metadata | its records by id, not wired |
| history read/write | yes (`/sync/history`) |
| playback read | yes (`/sync/playback`) |
| watchlist read/write | yes (`/sync/watchlist`) |
| favourites read/write | yes (`favorited_at`; write path to be read, not guessed) |
| custom lists read/write | yes (`/users/{id}/lists`) |
| ratings read/write | yes (`/sync/ratings`) — later slice |
| collection (ownership) | yes, but **on no list row**: it is not a list kind |

Settings show a switch only for a capability the provider actually has, named for
the thing it switches — "Browse it", "Describe titles", "Send what I watch",
"Bring in what I watched", "Send my watchlist", "Bring in my watchlist". Every
switch is off until chosen; any, all or none is a valid state, and a provider that
is off contributes to no merge in this document.

### 7.5 Echo suppression (revised after review)

The first draft assumed the outbox keeps a delivery time. It does not:
`TrackerState` holds only `done` + `failure`, and `prune` deletes delivered events
outright — so a time window over it suppresses nothing after the first drain, and
our retry ladder (1/5/30 min) can deliver well outside any 5-minute window.
Worse, the remote **restamps** on write: the fact we read back is newer than our
own `updated_at`, which is the classic "stale progress overwrites completed"
failure (JellyPlex #322).

The revised rule has three parts:

1. **Record the push when it is delivered, in a place `prune` spares.** A
   `deliveredAt` on `TrackerState` would be written and erased in the same
   `tracking::update` call (that closure ends by pruning). So `TrackingFile`
   gains a bounded, prune-exempt log:

   ```jsonc
   "pushed": [{ "delivered_at": "…", "imdb": "tt1", "title": "A Show",
                "kind": "series", "season": 1, "number": 2 }]
   // MAX_PUSHED = 64, trimmed on write at 24 h; `title` is there because
   // `same_target` falls back to the name when there is no IMDb id.
   ```

   written by the drain on `Delivery::Delivered` (`record_pushed`), and read by
   the import as `pushed_within(target, 24 h, now)`. Same target rules as
   `same_target`, so a film and an episode cannot mask each other.
2. **Skip a target with a pending outbox event** — it still has something of ours
   to say, and importing first would race the push. No such API exists today
   (`pending()` is a count, `due()` is per event+tracker), so add
   `TrackEvent::target()` plus `tracking::pending_for(file, &target)` surfaced as
   `heronite_watch::trackers::pending_for(app, &target)`; the import never parses
   the outbox itself. The skip must be **bounded by a clock the outbox keeps**:
   `TrackEvent.at` is when it *happened* (a backfilled session says 2019) and
   `Failure.next_at` moves per attempt, so `QueuedEvent` gains `queued_at` and the
   skip applies while `queued_at + PENDING_HORIZON` (36 min — the sum of the
   1/5/30-minute ladder) has not passed. A permanently `Unmatched` event never
   blocks a title.
3. **Never compare the provider's clock to ours.** The import compares provider
   timestamps to the *provider's own* previous snapshot (`remoteSeenAt` per title),
   and treats `pushedAt` only as "we recently wrote this, do not overwrite the
   local side".

## 8. Import algorithm

In provider terms: **change query -> compare snapshots -> pull only the axes that
moved -> apply -> save the snapshot last**. Simkl's `activities` and Trakt's
`last_activities` are the same step under different names, with different
granularity (Trakt has `collected_at`/`favorited_at`; Simkl has a
`removed_from_list` stamp); the core only sees `Changed` bitflags. A provider whose
change query cannot name removals (Trakt) falls back to the **diff** mode of §2.5,
which reads the whole list by ids and compares — the same shape as Simkl's
`simkl_ids_only` call, and just as expensive, so it only runs for a list whose
bucket moved.


```
import(provider):
  if signed out               -> refuse, "Sign in to <Provider>"
  if failure.nextAt > now     -> refuse quietly (row pressable, reason in its description)
  now  = provider.snapshot()             // Simkl: activities. Trakt: last_activities.
  axes = provider.changed(cursor.snapshot)   // our bitflags: history, playback, lists[kind]
  if axes.is_empty()          -> save lastPullAt; done (the cheap path)
  pulled = provider.pull(cursor.snapshot, axes)   // sequential, only the axes that moved
  // removal mode is the provider's (stamps | diff | none); a "diff" list is read
  // whole and compared with cursor.listIds only when its bucket moved.
  apply(pulled)               -- one lock per file, idempotent, watched facts never subtracted
  cursor.snapshot = now; cursor.save()   // last, and only if every call succeeded
```

- **Read the snapshot first, save it last, save nothing on failure** — Simkl's guide
  states it, and it is true of Trakt's `last_activities` for the same reason: a
  change made during the pull would otherwise be counted as already seen.
- Sequential requests: 10 GET/s and per-user quota are not the constraint, but the
  order makes failures diagnosable and avoids `max_items` pile-ups.
- One import at a time process-wide (`Mutex`), like the personal writes.
- Refusals are visible in the UI row (signed out, backoff, no provider) — the row
  stays pressable and says why, rather than going dead.
- **Automatic and periodic while the app runs** (the owner's decision, replacing
  the button-first draft). Triggers: app start, opening Watch, the end of a
  playback, a mark or list change, and a tick — default **every 15 minutes**, with
  jitter, backing off to 45-60 minutes while idle and shortening to ~1 minute right
  after local activity. Nothing runs when the app is closed.
- The tick is **one change-query request** when nothing moved. That is the quota
  budget: 96 requests a day at 15 minutes, 288 at 5 — inside Simkl's 500/day free
  allowance with room for writes, and the reason 15 is the default rather than 5.
  Simkl's guide discourages unconditional timers; tying ticks to the app being
  open, to user activity, and to the cheap change query is how this design stays
  inside that rule instead of ignoring it.
- Manual "Sync now" stays available as a fallback (in the row's actions), never as
  the only way to sync.
- The saved id set is maintained on **every** successful pull:
  `ids = (saved ∪ delta ids) ∖ (ids-only diff)`, and a delta row clears any pending
  removal — otherwise an add-then-remove between pulls is invisible.
- `/sync/playback`: when the playback bucket moved, the **whole open list** is
  re-read; imported positions for titles no longer in it are cleared.

## 9. Merge/normalization rules

The table is written in **our** terms; each provider's vocabulary is mapped in its
own notes (§6, §6b). Where a provider has no equivalent for an axis, nothing is
imported for it.


Applied per item, after identity mapping, under one lock:

| Remote fact | Local effect | Never |
|---|---|---|
| `completed` movie with `last_watched_at` | `finished = true`, `origin = imported`, `remote_updated_at` = the provider's stamp | write the provider's clock into `updated_at` (that field stays the local write time), or un-finish a local record |
| `completed` show, episode rows present | per-episode `finished = true` with remote dates in `remote_updated_at` | remove episodes the person watched here, or re-finish an episode marked unwatched locally |
| `watching` + `playback` % | `position_ms`/`duration_ms` computed from `progress` × the local duration, and only when the title was not recently pushed (§7.5) | overwrite a position on a locally `finished` record, or overwrite inside the push window |
| any enabled provider's membership of **any list kind** (`plantowatch`, a library, favourites, a custom list) | that kind's `in` **only if** local state for that kind is unset (§7.4) | flip an explicit local "out", mix kinds, or read absence as a removal |
| `hold` / `dropped` / `watching` | stored as a **remote annotation** (new `watch-personal.json` map `remoteStatus`) shown as one muted line | create a local state |
| ratings, memos, rewatches, custom lists | ignored in this plan | |

Conflicts: **union for watched**, **local for flags**, provider timestamps are
compared provider-to-provider, and `updated_at` is never compared to a provider's
clock at all. Within the remote stream, **completion dominates recency**: a remote
`completed` never becomes a position, and a remote position never touches a
locally `finished` record.

The rule this needs to stay honest: **a local record wins while it is younger than
the pull, for the position only** — `origin = local` and
`updated_at > cursor.lastPullAt` means the person used this machine since we last
looked, so the import leaves that position alone. It does **not** block the watched
union. Without the clause, "completion dominates" would silently discard a
position written five minutes ago in favour of a 2019 remote completion.

The un-watched marker is not episode-only: a film marked "not watched" locally is
`finished = false`, which rows 1–2 would re-finish on the next pull once it is
older than the pull. `MovieProgress` gets the same `unwatched` marker.

Every imported write records `origin`/`imported_from`/`imported_at` (snake_case in
`movies.json`, camelCase in `EpisodeProgress`) — but **never over a fact that is
already local**: re-painting `origin = imported` on a title the person marked here
would disable the "local wins while younger than the pull" guard for the next
import, which is both non-idempotent and dishonest on the page.

Two totality notes from the state review: the `1970-01-01` placeholder must be an
explicit branch of rule 1 (watched, no date), and rules comparing two providers
must compare **`remote_seen_at` per provider** — without it, applying the same
import twice or applying two providers in either order is last-writer-wins, not
"fresher progress wins" (§10).

One hole the review found in S1: a locally un-watched episode whose remote row is
still `completed` would be re-finished on the next import. S1 keeps a per-episode
local `unwatched` marker (provenance "local") that an import never overrides; it is
the same tri-state idea as the watchlist.

Imported facts are **ordinary history** once they land: a watched mark is a
watched mark, whoever wrote it. There is no "remove what came from <provider>"
control and no per-provider subtraction — signing out keeps what it brought in, and
provenance exists for the film page's muted "Watched · from <provider>" line and
for echo/logging, never for deletion. Un-marking a title is the person's own
action on that title, provider-agnostic.

## 10. Multi-provider semantics

- Imports are **per provider**, sequential, independent: a failed Simkl pull does
  not block a Stremio pull.
- The local store is the merge point: imports are additive (watched union,
  fresher progress wins), so a second provider cannot invent an owner.
- The cursor is per provider; provider removal = delete one file + optionally its
  imported records.
- **No cross-provider push of imported facts.** An imported watch is not
  re-sent; the outbox only carries local actions. If the person later watches an
  imported title here, that playback is what goes out (and Simkl's own dedupe —
  409 "already recorded" — handles the overlap).
- **A list change is a local action**, so it *is* sent — to every provider that
  can hold a list (§7.4). That is the mechanism, not a loop: imports never queue
  an event.

## 11. Why not the alternatives

| Alternative | Why rejected |
|---|---|
| Remote **mirror** files, merged at read time | a second truth to keep consistent; readers need canonical-id work; sign-out means subtraction; echo suppression harder |
| **Write-through** with no provenance | cannot distinguish "I watched here" from "Simkl says so"; cannot undo an import; cannot suppress our own echo |
| **Runtime merge view** over local + remote files | elegant, but puts canonical-id mapping and arbitration on every read; local files would still need provenance for undo. Reconsider if a second *reader* (e.g. a stats page) needs remote-only facts |
| Continuous **reconcile daemon** | the provider docs explicitly warn against unattended polling; conflicts have no UI; cost unbounded |
| Import via **search** (`/search/id`) | needs a user token, doubles requests, and the docs call it a last resort — we already replaced it with `/redirect` in metadata |
| **Title+year** matching | silent wrong-title imports |

## 12a. Code facts the review corrected

These are not design choices; they are things the first draft assumed wrongly:

- ~~`UreqTransport::new()` caps a body at 1 MiB~~ — the import builds its
  transport with `with_limits`, so the baseline comes through without touching the
  parser. **Done.**
- ~~`get_bearer` keeps only 2xx bodies, so `400 max_items` is invisible~~ —
  `Transport::get_bearer_answer` exists (status + body, `bail!` default,
  implemented by `UreqTransport`, with `get_bearer` delegating to it). **Done.**
- ~~`playback::record` rewrites `movies.json` outside the lock~~ — **Done:**
  `store::update_movies` wraps `json_store::update`, playback and
  `progress::set_watched` both go through it, and the misleading comment is
  replaced by the real rule (one non-reentrant per-path lock).
- Continue watching today is **Stremio's** rail, whose own catalog id is
  `continue-watching`; `continue:<provider>` is the *settings* id built from the
  media providers. A local rail needs its own id and an honest description —
  "what you started there" would be a lie once imported progress mixes in.
- `MovieProgress` serializes in `snake_case` while most Watch types are camelCase;
  new provenance fields must match the file they live in. And unlike most Watch
  types it has **no `#[serde(flatten)] extra`**, so a downgraded build would drop
  those fields on its next write — add the `extra` map (§18.8 D2 is wrong without
  it).
- `watch-simkl` has no `heronite-data` dependency, so the cursor file's paths and
  atomic writes belong to `heronite-watch-state`, not the provider crate.

## 12. Testing plan

- **Cursor/planner** (pure): given an activities snapshot pair, which endpoints
  are planned; no-op when `all` matches; `removed_from_list` → ids diff only.
- **Merge rules** (pure functions over fixtures): union of watched, newest-wins
  with the echo window, tri-state watchlist, provenance and un-import.
- **Transport**: fake `Transport` (existing pattern in `crates/watch/tests/`),
  fixtures from the guide's JSON examples; a real-socket test only for the
  redirect/status handling (already established in `watch-core`).
- **Failure matrix**: 401 (sign in again, no retry loop), 429 with `Retry-After`,
  `400 max_items` (split), network fault (backoff, snapshot untouched),
  mid-pull crash (no partial snapshot).
- **Determinism**: injected clock everywhere (`Utc::now()` is already threaded in
  most places); no sleeps in tests.
- **Rails**: Continue watching reads imported progress; empty providers change
  nothing.

## 13. Performance and cost

- First pull: ~4–6 sequential GETs (all-items per type, playback, ids), tens of MB
  worst case for a huge library with episode flags.
- Steady state: 1 GET when nothing moved; a handful when something did.
- Reads: no per-frame file access — the local store is read once and cached in
  memory (pattern already used by `watch-personal`), invalidated on import/write.
- Cursor files are kilobytes.

## 14. UX (revised after review)

- **Two independent settings, never one.** `trackers` (send) and a new import list.
  Turning import on must never queue a history backfill — that is what `set_tracker`
  does today and it must not be reused. Rows interpolate the provider
  (`tr!("Send what I watch to {provider}")`, `tr!("Bring in what I watched in {provider}")`)
  rather than baking a service name into a key, and both are off by default.
- **Automatic, with a visible status line**: the row shows "In step · 4 minutes ago" /
  "Bringing in 3 titles…" / "Simkl removed 1 title" rather than an action the
  person must remember. "Sync now" is a secondary action.
- **The card exists before sign-in.** Today the Trackers group is built from
  configured *and signed in*, which would make the import switch unreachable and
  "signed out" unshowable. Gate the group on the build knowing the provider, and
  let the description carry the state.
- **Reuse the app's existing vocabulary.** `pages/settings/launchers.rs` already
  says "Bring its games in" / "Bring in now" / "Bringing in…" / "Remove them" —
  the same words here: **"Bring in now"**, **"Bringing in…"**.
- **Feedback is the row, dated** (the pattern `tracker_state` already uses), not a
  toast. The sentence is composed from `trn!` pairs rather than a fixed key —
  `{n} titles`, `{n} episodes`, `· {when}` — because plural rules differ per
  locale and the i18n gate compares keys exactly. A refusal keeps the row
  pressable: "Simkl could not be asked: trying again in 12 minutes."
- **Promise only what happens**: "The watched marks and positions Simkl keeps, for
  the titles you already have here." No mirror, no new titles in the library.
- **Sign-out keeps what came in** and says so: "Signed out of Simkl. What it
  brought in stays." No destructive question and **no "remove what came from
  <provider>" row** — imported marks are ordinary history (§9); a title leaves the
  list the way every other title does, by the person un-marking it.
- **The film page gains no elements**: the existing watched line reads
  `tr!("Watched · from {provider}")` when the whole mark came from that provider
  (today `title_status` returns a bare `tr!("Watched")`, so the provenance key is
  new, not a reuse); per-episode rows say nothing, and mixed marks say nothing.
- The daily-on-open option is named and its cost stated, because something will
  appear with no press: "Bring in when I open Watch" — "At most once a day, and
  one request when nothing changed."

## 15. Rollout

1. **S1 — playback + history** (this plan's core): cursor file, `/sync/playback` +
   `/sync/all-items` deltas, normalization into `movies.json` with provenance,
   Continue watching reads the result. No watchlist.
2. **S2 — watchlist**: tri-state flag migration, `plantowatch` in when unset,
   explicit toggles out (`removed_from_list`), watchlist rail.
3. **S3 — annotations and deletions**: remote status line, `dropped/hold`
   surfacing, deletion reconciliation notices, per-provider rail switches.
4. **S4 — Simkl as a catalogue**, TMDB's shape rather than a second kind of
   browsing. Free, edge-cached, exempt from both limits: **trending**
   (`/discover/trending/{type}/{file}.json`, and the combined
   `/discover/trending/{file}.json`; `file` is `today|week|month` × `_100|_500`)
   and the **calendar** (`/calendar/v2/{type}.json` with `type` = `tv`, `anime`
   or `movie_release` — not `movie`). Token-gated **and quota-counted** (not
   exempt): **search** (`/search/{type}`) and the account lists
   (`/tv|anime/best/{filter}` with `month|all|year|voted|watched`,
   `/tv|anime/airing`, `/tv|anime/premieres/{new|soon}`). Signed out it declares
   the file rails only; the account rails appear with the account and leave with
   it. Search debounces and caches per session (Free plan: 500 requests/day).
5. **S5 — favourites and custom lists.** The same ledger and the same rules, per
   kind: favourites wherever the provider has them (Simkl's is beta and PRO/VIP),
   custom lists only through an explicit local↔provider binding (§7.4). The
   capability matrix grows a column per kind, and the settings rows name the kind:
   "Send my favourites to {provider}", "Bring in my custom list '…' from {provider}".
6. **S6 — anime trackers** (AniList / MyAnimeList / Simkl's anime side), a slice
   of its own: anime has its own identity (`mal`, `anidb`, `anilist`, `kitsu` —
   often **no IMDb id**), its own status vocabulary and its own lists. The main
   design stays untouched: a tracker is a tracker, and `canonical()` gains an
   anime branch when that slice lands. Until then anime titles are counted as
   no-IMDb skips (§18.9.5), never guessed.
7. Later: ratings, rewatches; Stremio datastore import (mostly a cross-device
   catch, since we already push there).

## 16. Open questions (review answers in §17)

1. Is provenance on the local record the right place for "who watched this", or
   should the import write a separate *event* file (like sessions) and derive?
   Trade-off: derivation is cleaner but pushes work onto every read.
2. ~~Is the per-title `wroteAt` window (5 min) enough?~~ Replaced by the delivery
   log (§7.5). What remains open: whether Simkl's `409 already_watched.watched_at`
   or Stremio's `_mtime` should be kept as the authoritative "we wrote this" mark
   instead of our own delivery time.
3. Should `hold`/`dropped` be annotations only, or is there a local state we are
   missing?
4. Is the "Import now" + optional daily-on-open model enough, or do people expect
   a "reconnect and it just appears" feel? How do comparable apps do it?
5. S1 writes into `movies.json` under one lock; is a whole-file rewrite acceptable
   at 5k titles, or does this need a per-title file/shard?
6. `ids` diff for deletions: is remembering the Simkl id set the right base, given
   items can be removed and re-added between pulls?

## 17. Review log (2026-11-18)

Four independent reviews of the draft: data model / Rust, Simkl protocol (checked
against the live docs), UX/product, and a comparison with comparable apps.

**Accepted and folded in above (blockers):** identity needs `extended=ids_only`;
`include_all_episodes` is required for completed/dropped deltas and synthesizes
dates; echo suppression cannot use a delivery time the outbox does not keep — it
now uses a per-title `pushedAt` written at delivery plus "skip titles with pending
events", and never compares the provider clock to ours; `/sync/playback` has
percentages, no duration, and disappears when a watch lands; the transport's 1 MiB
cap and `get_bearer`'s body-dropping block the baseline and `max_items` detection;
`playback::record` is outside the locked update; Continue watching is Stremio's
rail, so imported progress needs a local rail; the watched mark needs a local
"unwatched" marker so an import cannot re-finish it; the 429 taxonomy
(`rate_limit` vs `user_limit_exceeded`), token expiry/refresh, the 1970
placeholder, `notinteresting → dropped`, and the `max_items` split ladder are all
now in §6.

**UX corrections folded in:** two independent switches (send ≠ import) with the
send row named by thing + provider; the card must exist before sign-in; reuse
"Bring in now" vocabulary; dated row instead of a toast; sign-out keeps imported
data; the film page reuses its existing watched line. (The "remove what came from
<provider>" row this round proposed was dropped in round 4 as an invented
destructive control.)

**From the research review, adopted as rules:** watched is a one-way ratchet (un-play
is opt-in and out of scope, §9); completion dominates recency; deletions only ever
withdraw *list membership*, and the escape hatch is the removal row; provider
dedupe is weak (Trakt: "up to your app"; Simkl: `409 already_watched`) so the client
owns idempotency — our outbox already does. Research also notes mature tools ship
a scheduler and an import *report/mode*; we deliberately skip the scheduler (§5)
and adopt a small report (`applied / unchanged / skippedNoImdb / refused`) because
it is what makes a silent failure visible.

**Left open:** whether Simkl's `shows` bucket moves for an episode-only watch (an
API claim not yet verified live); whether an explicit "take remote as truth" mode
is worth a second switch, or is just "Bring in now" plus the person un-marking
titles; whether `movies.json`'s whole-file rewrite holds at 5k titles with imports
added (the review says keep it and fix the unlocked writer, not shard).

### Round 2 (local-only, 2026-11-18)

The protocol re-check ran with file tools only. Its first finding was operational:
**subagents cannot see `/tmp`** — their sandbox has its own scratch directory — so
"use the local copies" requires the copies to live **inside the workspace**. The
Simkl reference set is now cached at `.cache/simkl-ref/` (gitignored), and the
re-run reads from there. No network tool is used by review tasks; the research
task is the single, explicit exception.

The revisions review found three more blockers, all folded in above:

1. **`pushedAt` had nowhere to live.** `delivered()` and `prune()` run inside the
   same `tracking::update` closure, so a `deliveredAt` on `TrackerState` would be
   erased in the same call. Hence the prune-exempt, bounded `pushed` log (§7.5).
2. **"Skip a title with pending events" had no API and no bound.** `pending()` is a
   count; `due()` is per event+tracker; a permanently retrying event would block a
   title forever. Hence `pending_for(target)` plus a time-bounded skip (§7.5).
3. **Two clocks in one field.** Writing the provider's stamp into `updated_at`
   contradicts both §6's "never shown as an exact watch time" and §7.5(3)'s
   provider-to-provider comparison. Hence `remote_updated_at`, and the "local wins
   while younger than the pull" clause that keeps completion-dominates honest.

Two wording corrections from the same review: `MovieProgress` is snake_case (so
the provenance fields are too — and must not be renamed wholesale), and the
Continue-watching id is `continue-watching` on the provider with `continue:<provider>`
being the settings id.

**Still open:** whether Simkl's `shows` bucket moves for an episode-only watch; if
`skippedPending` belongs in the report (`§17` research note says yes); and whether
the `.cache/simkl-ref/` set should gain the `list-statuses` page, which the
protocol review needs for the `notinteresting → dropped` claim.

### Round 3 (local-only, workspace cache)

The workspace cache (`.cache/simkl-ref/`) is visible to subagents where `/tmp` is
not — but it is **gitignored**, so a concurrent `git clean -xdf` wiped it mid-run;
refill it before a review (the URLs are in §6). Corrections from this round:

- **The default id shape was wrong in the doc.** `/sync/all-items` without
  `extended` *can* carry `imdb` (the OpenAPI example does); it is guaranteed only
  with `ids_only`/`full`, and **anime** is the realistic failure case. The rule is
  unchanged — ask for `ids_only` — but the reason is now accurate.
- **Playback positions are approximate**: `progress` rounds to an integer, and
  sessions expire by plan (7/30/90 days) without notice.
- **The skip's clock had no source**: `TrackEvent.at` is when it happened, not when
  it was queued — hence `queued_at` + `PENDING_HORIZON`.
- **Two clocks, again**: the "local wins" clause is scoped to positions (it must
  not block the watched union), and **films** need the `unwatched` marker too.
- Smaller: `pushed` entries carry `title`; `MovieItem` provenance is `imported_at`
  (snake_case); `store::update_movies` does not exist yet; the path lock is
  non-reentrant; and §14 must interpolate `{provider}` everywhere — deleting the
  old "Simkl"-baked keys from all four locales, or the coverage gate fails on
  orphans.

### Round 4 (owner review)

Three corrections, all folded in:

- **The watchlist is not Simkl's.** §7.4 is provider-agnostic now: the local list
  is *the* list, every provider that has one is an input or an output (any, all or
  none), and the two-accounts-plus-local case is written out step by step.
  Favourites stay local because no shipped provider has them.
- **"Remove what came from <provider>" is gone.** It was an invented destructive
  control; imported facts are ordinary history, sign-out keeps them, and provenance
  is for the display line and echo logic only.
- **Simkl is a full catalogue, not just metadata.** S4 adds trending + calendar
  rails on the free files and search + `best`/`airing`/`premieres` with the account
  — TMDB's shape, degrading to the file rails when signed out.

## 18. Scenarios we must cover

Each row is a concrete case, the behaviour this design already decides, and — where
it decides nothing — a **GAP** number (listed at the end with a recommendation).
"Provider" means any media provider or tracker; none is special.

### 18.1 Watching on this machine

| # | Scenario | Expected |
|---|---|---|
| W1 | Film played to the end | local `finished`, session logged, outbox event `Watched` |
| W2 | Film stopped at 40 % | local position, `EpisodeProgress`-less record, event `InProgress` → `/scrobble/stop` 40 |
| W3 | Episode finished, next one not | that episode's record `finished`; the show's own `finished` stays false |
| W4 | Rewatch of a finished film | position resets, `finished` false until it passes the threshold; the session log keeps both runs |
| W5 | Mis-click (< 45 s) | no session, no event |
| W6 | mpv fails to start | no session, no event, the page says so |
| W7 | Playback while offline | everything local; events queue and retry |
| W8 | Two players at once (a stream and a trailer) | refused by the "one player" rule, not by the sync layer |

### 18.2 Manual marks

| # | Scenario | Expected |
|---|---|---|
| M1 | Mark a film watched | local write + `Watched` event + toast |
| M2 | Un-mark a film that a provider also marked | local `unwatched` marker; the next import must not re-finish it (§9) |
| M3 | Mark an episode | per-episode `finished`; the show is not marked |
| M4 | Mark a series (all episodes) | not offered on a series row — the person marks episodes (today's rule) |
| M5 | Mark while an import is running | **GAP 1** |

### 18.3 Sending (the outbox as it exists)

| # | Scenario | Expected |
|---|---|---|
| O1 | No tracker enabled | local only, nothing queued |
| O2 | Tracker enabled, signed out | not "enabled"; nothing queued |
| O3 | Tracker switched on for the first time | backfill of finished sessions is the first sync (today's behaviour) — see **GAP 2** |
| O4 | Tracker switched off with events pending | events are kept for it (prune spares it) and delivered if switched on again |
| O5 | Provider refuses permanently (`Unmatched`) | dropped once, one notice, never retried |
| O6 | Provider says `400 RATE_LIMIT` (write lock) | retry shortly — *not* a quota |
| O7 | `429 rate_limit` (burst) | retry ~1 s, ignore `Retry-After` |
| O8 | `429 user_limit_exceeded` | wait `Retry-After` (next US-Eastern midnight) |
| O9 | `401` | stop, "sign in again", keep the event |
| O10 | Same film watched twice on this machine | folded per title: the newest state is delivered once |
| O11 | Event older than 24 h / outbox over 500 | pruned / oldest dropped (`MAX_EVENTS`) |
| O12 | Simkl answers `409 already_watched` | treated as delivered |
| O13 | Two trackers, one accepts and one retries | per-tracker state; the event stays until all enabled trackers are done |

### 18.4 Importing

| # | Scenario | Expected |
|---|---|---|
| I1 | First import, empty local | Phase 1 pull; watched marks and positions appear |
| I2 | First import, local history already there | union watched; positions only where local is older *and* not finished (§9) |
| I3 | Nothing changed remotely | one `activities` call, stop |
| I4 | Only an episode changed | delta with `extended=full&episode_watched_at=yes`; only that title touched |
| I5 | Huge library / `400 max_items` | split type → status → `original` (GAP 6) |
| I6 | Network dies mid-pull | no snapshot saved; next run repeats the same `date_from` |
| I7 | Import pressed twice | one import at a time process-wide (§8); the second press is a no-op and the row says "Bringing in…" |
| I8 | Signed out / no client id | refused with the reason on the row |
| I9 | Token expired (7 days) | refresh before pulling; else 401 → sign in (GAP 7) |
| I10 | Daily quota spent | stop until `Retry-After`; nothing lost |
| I11 | Remote removed a title from its list | membership only; watched facts stay (§9) |
| I12 | Remote playback session vanished | imported position cleared for that title (§8) |
| I13 | Two imports of different providers at once | serialised per provider; independent failures |
| I14 | Sign-out during an import | abort, no snapshot save |
| I15 | Corrupt cursor file | treated as "no cursor": one full pull |
| I16 | A title watched remotely but **not in the local library** | a greyed placeholder appears (uninstalled-game style), pressing it says it is not available here |
| I17 | A title with no IMDb id (anime) | counted as a skip in the report (GAP 5) |
| I18 | Remote status `hold` / `dropped` | annotation line only, no local state |

### 18.5 Lists (watchlist + favourites)

| # | Scenario | Expected |
|---|---|---|
| L1 | Local only, no provider lists | unchanged behaviour |
| L2 | Add locally, one provider | pushed as `in` |
| L3 | Provider had it, local unset | imported as `in` |
| L4 | Local `out`, provider still has it | not re-added; the removal is sent |
| L5 | Removed on the phone | stays locally (§7.4 asymmetry) |
| L6 | Two providers disagree | local is the union; the next local change re-sends |
| L7 | Favourite toggled | local only, never sent |
| L8 | Provider removed from settings | its imported entries stay; the cursor file is orphaned (GAP 8) |
| L9 | Re-add a title removed locally | the remote add clears it too now (lists mirror, §7.4); no special "reset" needed |
| L10 | A provider with no list capability (Cinemeta) | no switch, no merge, no error |

### 18.6 Composition and identity

| # | Scenario | Expected |
|---|---|---|
| C1 | Same title watched on Stremio and on Simkl | one local record: union watched, newest position within the remote stream |
| C2 | Two providers' episode shapes differ | matched by `(season, number)` inside the title (GAP 11) |
| C3 | A provider changes its own ids | `imdb` is stored with every record for this reason |
| C4 | Title renamed remotely | the local name wins; the recap keeps the played name |
| C5 | Provenance: the same fact from two providers | single `origin` field today (GAP 12) |

### 18.7 Catalogues, metadata, trailers

| # | Scenario | Expected |
|---|---|---|
| K1 | Simkl catalog signed in | trending + calendar rails, search, best/airing/premieres |
| K2 | Simkl catalog signed out | trending + calendar only; account rails absent with a reason |
| K3 | Search typed fast | debounce, cache per session (quota is per user) |
| K4 | Metadata priority reordered | refetch (cache key is the ordered set) |
| K5 | Trailer is a vertical Short | banner is the backdrop or a wide crop; no portrait frame |
| K6 | No trailer | no `L3 · Play trailer` hint on the title's page |
| K7 | Catalog source off | it declares nothing; no rows, no errors |
| K8 | Trailer from a cover | `L3` plays the focused title's trailer; its details are asked for first when the page was never opened |

### 18.8 Data safety and clocks

| # | Scenario | Expected |
|---|---|---|
| D1 | File corrupt | default value + one log line; the space still starts |
| D2 | Downgrade to an older build | unknown fields survive (`extra`), known fields keep values |
| D3 | Local clock is wrong (ahead) | **GAP 13** |
| D4 | Local clock behind | a remote position can look "newer" and win — the push log is the guard |
| D5 | `1970-01-01` remote date | watched, no date, sorted last |
| D6 | Same timestamp on both sides | local wins (deterministic tie-break) |
| D7 | Imported record then watched here | local write wins; the event goes out; the remote echo is suppressed |

### 18.9 The GAP list

1. **Mark while an import is running** — two writers on `movies.json`. Today
   `playback::record` is unlocked anyway (§12a); the import must take the same lock
   and the mark must see a consistent file.
2. **Switching a tracker on backfills all finished sessions**, which makes every
   one of those titles "pending" — and I7 says imports skip pending titles for
   `PENDING_HORIZON`. Either import runs first, or backfill events are exempt from
   the skip (`at` is old anyway). Needs a rule.
3. **Double import / import spam** — the mechanism exists (§8's process-wide
   guard); what is missing is only the row's "Bringing in…" state.
4. ~~A title watched elsewhere that is not in the local library.~~ **Decided
   (owner): it appears**, as a greyed placeholder in Continue watching — the same
   language the collection already uses for a game that is not installed
   (`theme.tiles.grey_uninstalled`). The import creates a local item from the
   provider's own data (title, year, poster, imdb, kind) with no playable source;
   pressing it explains that it is not available here rather than failing to play.
   It turns real when the library gains the title. Open detail: when a placeholder
   disappears from every list *and* was never watched here — remove it, or keep it
   as history? (It carries no local watch by definition, so removal is safe.)
5. **No-IMDb titles** — skipped, counted. Anime is the common case; if we ever
   want them, the fallback is `(title, year)` with the wrong-title risk.
6. **`max_items` splitting** is described but not specified per request type; the
   `original` fallback also returns fewer rows than `watched_episodes_count`.
7. **Token refresh** — `auth.rs` has no refresh path yet; the import needs one, or
   it will fail every 7 days.
8. **Provider removed from settings** — cursor orphaned, imported facts stay.
   Nothing cleans it up; is that acceptable? (I think yes; say so.)
9. **List mirroring conflicts** — covered by the three-way table in §7.4; what
   still needs a test is the pair "added here, removed there in the same window"
   and "two providers disagree inside one tick" (newest bucket time wins, local on
   a tie).
10. **The tick's cost over a month** — 96 change queries a day per provider is
    cheap for one, but not free for four: the budget must be shared (a global
    interval, not one per provider) or the free tier runs out. Not yet decided.
11. **Episode identity across providers** — local ids are `tt…:1:1` for Stremio;
    a remote row is `(season, number)`. The map is inside the title; write it down.
12. **Provenance is a single value.** With two providers importing the same fact,
    `origin` holds the last one. If the film page must say "from Simkl" honestly,
    it needs a small source *set*, or the line must not name a provider.
13. **Local clock ahead** breaks "younger than the last pull" permanently. Cap the
    window (e.g. 1 h) rather than trusting an unbounded `updated_at`.
14. **The same word means different things.** A provider's "collection" is not a
    watchlist, its "favourites" may be a star rating, and MAL's "plan to watch" is
    not Simkl's `plantowatch` in every detail. Each provider crate must declare a
    **kind mapping**, and the settings must name the kind, not the provider's word.
15. **A gated kind must fail visibly.** Simkl favourites/custom lists on a free
    account answer `403 premium_only`: the switch cannot look broken, and the
    refusal must not be retried as a transient fault.
16. **Custom-list identity.** Binding is explicit, so the state to keep is the
    binding itself (local list ⇄ provider list id), and what happens when the
    provider deletes or renames the list — detect and mark the binding stale, never
    silently re-create it.
17. **Ordering inside a list.** Local order is the person's; providers may not keep
    it. Decide once: local order wins on display, provider order is not imported.

## 19. User stories

Read each as: **given** that state, **when** that happens, **then** this is what a
person sees and what moves on disk or on the wire. Provider letters are places:
**S** = Stremio (media provider), **B** and **C** = two trackers, each with a
watchlist and a history; **L** = the local library.

### 19.1 First contact

**US-01 — Nothing connected.**
Given a fresh profile (every source off but Stremio, not signed in).
When the person opens Watch. Then the space opens with no catalogs, no metadata, no
marks, no errors, and one honest next step per page (Accounts: sign in; Sources:
nothing on). *Nothing is broken because nothing is chosen.*

**US-02 — Stremio only.**
Given S linked, catalog and metadata off.
When the person plays a film to the end. Then the local record and session exist;
the film page says Watched; **nothing is sent** (no tracker) and nothing is
imported (no importer). With a catalog source switched on, the same film appears in
Movies; with metadata on, its page has facts and a trailer hint.

**US-03 — Simkl, import only, empty local.**
Given B connected with **send off, import on**, and L empty.
When a sync tick runs. Then B's history lands in L (watched marks with B's dates,
positions as hints), the row says "In step · 12 titles brought in", and titles the
library does not have appear in Continue watching as **greyed placeholders** — the
same treatment as a game that is not installed — so they can be recognised without
pretending they can play.

**US-23 — Added here while the provider does not have it.**
Given a title added to the local watchlist, absent from B's list.
When a tick runs. Then B receives the add; the local entry stays. Absence was
*never there*, not *removed* — the last-seen set (§7.4) is what tells them apart.

**US-24 — Removed on the phone.**
Given a title in both lists.
When the person removes it in B's app and a tick runs. Then it leaves the local
list too ("B removed 1 title"), and the removal is sent to any other provider that
still lists it. The same title's *watched* mark is untouched: history is one-way.

**US-04 — Import never pushes.**
Given US-03's state.
When the import finishes. Then the outbox is still empty: imported facts are not
re-sent. Switching **send on** afterwards does not queue them either (only a local
action does); its first-sync backfill covers *sessions logged here*, not the
imported ones — unless we decide otherwise explicitly.

### 19.2 Watching, then connecting

**US-05 — No history, connect B, watch, connect C.**
Given L empty, B connected (send on).
When they watch three titles, then connect C (send on). Then the three are pushed
to B as they happen; C's first sync **backfills every finished session in L**, so C
receives them too; no duplicates on B. Import from C is a separate, later action.

**US-06 — C already has a history.**
Given the same, and C has five watches from another device.
When C is connected and the first tick runs. Then L gains those five (greying the
ones it cannot play), positions only where L is older and not finished, and nothing
is pushed back for them.

**US-07 — A new tracker with an old list.**
Given L's watchlist has two titles, C has ten.
When C is connected and imported. Then L shows twelve (union, `in`), C receives the
two it lacked **only when the next local list change happens** (import does not
push), and nothing is removed from either.

### 19.3 Two trackers and the local list

**US-08 — The three-way list.**
Given L has *A, B*; B has *B, C*; C has *A, D* — all with lists enabled.
When everything is imported and then used. Then: L becomes *A, B, C, D*; the next
local change sends the missing ones to each side; removing *A* locally removes it
everywhere; removing *C* in B's app **changes nothing here** and it stays in L
until the person removes it locally (§7.4). Favourites never move at all.

**US-09 — The person disagrees with the union.**
Given US-08 after B was cleared in its own app.
When they look at L. Then the title is still there — deliberate — and the row's
description says what came from where. Un-marking it here removes it everywhere.

### 19.4 Removing and signing out

**US-10 — Remove locally, then import.**
Given a title marked `out` locally.
When an import runs and B still lists it. Then it is **not** re-added (an explicit
local decision beats a remote fact), and no removal is resent unless the local
state changes again.

**US-11 — Sign out of B.**
Given B connected and imported data present.
When they sign out. Then sends stop, the import switch leaves with the account,
tokens are cleared, **the imported data stays** ("Signed out of B. What it brought
in stays."), and there is no destructive question. Reconnecting resumes with a new
first pull (the cursor is per provider).

**US-12 — Turn B off in Sources.**
When both switches are off. Then no sends and no pulls, the imported facts remain,
and the cursor sits unused (orphaned by design); turning it back on resumes from
that cursor or from a full pull if the file was lost.

### 19.5 Failures and recovery

**US-13 — Offline for a day.**
Given three watches, one mark and one list removal while offline.
When the network returns. Then the outbox delivers them in order, one at a time,
with the 1/5/30 ladder; **no import happens by itself** unless "bring in when I
open Watch" is on; nothing is lost, nothing is doubled.

**US-14 — Token expired mid-week.**
When a send gets `401`. Then the event is kept, the row says "sign in again", and
further sends stop until sign-in; after signing in the queue flushes. An import in
the same state refuses with the same reason instead of retrying (GAP 7: no refresh
path yet).

**US-15 — Import pressed twice.**
When the second press lands while the first runs. Then the second is a no-op with
a reason ("Bringing in…") — one import runs at a time (§8).

**US-16 — Pull dies mid-way.**
Given a large import and a dropped connection in the third request.
When it fails. Then **no snapshot is saved**, the cursor is byte-identical, and the
next attempt sends the same `date_from` — the changes made during the failed run
come back.

### 19.6 Two machines, one title

**US-17 — Watched in B's app, then here.**
Given B marks a film watched two hours ago.
When the person watches the same film here today. Then the local record is the
newest and wins; the event goes out; B's echo (its `last_watched_at`, now newer than
ours) is ignored inside the push window (§7.5), and a later import does not
downgrade a finished record.

**US-18 — Half-watched on two devices.**
Given B has 30 % and L has 70 % (written yesterday).
When an import runs. Then L keeps 70 % (newest **within the remote stream** for
remote facts; a local write older than the pull is fair game) — and the position
imported is described as a hint, not a bookmark, because B's `progress` rounds.

### 19.7 Catalogues, metadata, anime

**US-19 — Simkl as a catalogue.**
Given Simkl metadata on and the account signed in.
When they open Movies. Then trending today/week rails and the calendar are there
from the free files; search and best/airing/premieres are available because the
account is signed in; typing debounces and caches. Signed out, the file rails
remain and the account rails leave with the account.

**US-20 — Anime, later.**
Given an anime tracker (AniList/MAL) in a future slice.
When it is connected. Then it behaves like any other tracker — send, bring in,
lists — but with its own identity keys (`mal`, `anidb`, `anilist`); the main paths
are unchanged because no path knows a provider by name. Until then its titles are
counted as no-IMDb skips, never guessed.

**US-22 — Trakt and Simkl side by side.**
Given both connected (history, watchlist, favourites), and a local library that
predates both. When both are brought in. Then each is asked in its own vocabulary —
Trakt's change query and its plays, Simkl's activities and its all-items — and both
land as *our* history, positions and memberships: one local watchlist, one local
favourites list, one set of watched marks. Trakt's **collection** is not imported
(ownership is not a list, §6b). A removal in Trakt is learned by diffing its
watchlist, the same rule Simkl uses for its ids; the local list is unaffected either
way (§7.4).

**US-21 — Trailer flow.**
Given metadata sources on and a title with a trailer.
When the film page opens. Then the hint bar offers `L3 · Play trailer`, which plays
in mpv and records nothing; with no trailer the hint stays what it was; a vertical
Short shows as a wide banner or a pillarboxed player frame, never a portrait.

### Round 5 (four reviewers, local-only)

Lenses: state-machine completeness, adversarial walk of the user stories, an
implementation plan against the code, and a local verification of the Simkl
catalogue/list claims.

**Verified Simkl corrections, applied:** trending is `today|week|month` ×
`_100|_500` (plus a combined file); calendar `type` is `tv|anime|movie_release`;
search and the account lists are namespaced under `/tv`/`/anime`, are **token-
gated, uncached and quota-counted** (the enums are `month|all|year|voted|watched`
and `new|soon`); and Simkl **does** have favourites (`/lists/favorites`, beta,
PRO/VIP) — so "no provider has favourites" was wrong. Anime sync payloads carry
`mal`/`anidb`/`anilist`/`kitsu` and **no IMDb**, confirming §7.3's skip rule.

**Merge defects found:** rule 1 is not total (the 1970 placeholder has no branch);
rules 1–2 re-paint `origin = imported` over a genuinely local mark, which disables
the local-wins guard and breaks idempotence; rules comparing two providers have no
per-provider watermark (`remote_seen_at`), so order decides the result. All three
are now stated in §9.

**Things the code cannot support as written:** the tri-state cannot live on
`PersonalLibrary` entries (removal deletes them) — it is a separate ledger; the
cursor must be owned by `watch-state` (`watch-simkl` has no `heronite-data`); and
`MovieProgress`/`EpisodeProgress` need a `serde(flatten)` `extra` or a downgrade
silently loses provenance.

**The two product decisions left:** GAP 4 (may an import create a title the library
does not have?) and whether a remote list removal may ever remove a local entry.
The state review argues **against** creating titles — a local rail joined by IMDb,
plus a counted "N watched remotely, not in your library" line — while the story
walk shows the current stories ("B's history lands in L") are unfulfillable for an
empty library without it. One of the two has to give.

### Round 6 (owner correction: lists are three kinds)

The owner pointed out what the document kept shrinking: **most providers have
favourites, a watchlist and custom lists**. §7.4 now models three kinds with a
capability per kind per provider, and the Simkl facts are verified locally: the
watchlist is `plantowatch` (free), while `/lists/favorites` and `/lists/{id}/items`
are **beta and PRO/VIP** (`403 premium_only` on free). That kills two earlier
claims — "favourites are local only" and "no shipped provider has favourites" — and
turns favourites/custom lists into slice **S5**, with an explicit local↔provider
binding for custom lists (never name matching) and a visible refusal for a gated
kind.

### Round 7 (owner: stop building around one API)

Trakt's blueprint was read from the local cache (`.cache/trakt-ref/trakt.apib`) and
compared with Simkl's. The *shape* is shared — a change query with timestamps,
history with per-play ids, playback with `progress`, a watchlist, favourites, custom
lists, ratings — but almost nothing else is: different vocabulary, removals by diff
only, app identity in **headers** rather than query params, and a `collection` that
is ownership rather than a list. Consequences folded in: §2.5 is now the neutral
contract (the core sees change queries, snapshots and capability axes); §6 and §6b
are provider notes; §8 speaks in those terms; the matrix has a Trakt row and an
explicit "collection is not a list kind"; US-22 walks both providers at once.

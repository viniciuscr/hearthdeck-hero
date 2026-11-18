# Watch trackers — the local record, and who follows it

Status: implemented slice 1 (Simkl). `docs/WATCH.md` owns the feature; this
document owns how a **tracker** is told what happened, and what a screen shows
about a title's progress.

A tracker is not a provider. It plays nothing, it owns no titles, and it is never
on the way of a playback: it keeps a person's watch history in step with a
service (Simkl today). The local record is the truth and the tracker follows it —
when the network is there, not when the film ends.

## The three states

A title's (or an episode's) state is **derived**, never stored, from the position,
the length and the `finished` flag in `movies.json`
(`heronite_watch_core::progress::WatchedState`):

| State | Rule | On screen |
|---|---|---|
| `Unwatched` | no record, or under **one minute** played | a play mark; nothing said |
| `InProgress` | one minute or more, under **70 %** | the half mark (`icons/progress.svg`), "In progress · 40 %", "Resume from here · 40 %" |
| `Watched` | `finished`, or 70 % reached | the tick |

70 % is Stremio's own threshold (`WATCHED_THRESHOLD_COEF`), so a title watched in
the provider counts as watched here. A `finished` flag that came from a source
wins over the arithmetic. A series has no title-level state: it counts its
episodes, and the header reads "3 of 8 episodes watched" when the page knows the
episode list ("3 episodes watched" when it does not).

`EpisodeProgress` keeps `season` and `number` beside its position, filled from the
page's episode list. That is what a tracker is given — the provider's `video_id`
(`tt0944947:1:2`) is a shape of its own and is never parsed.

## What is written, and when

| File | Holds | Written by |
|---|---|---|
| `movies.json` | the title's position, length, `finished`, `video_id`, per-episode records | playback, and **a mark by hand** (`progress::set_watched`) |
| `watch-sessions.json` | one record per playback: start, end, coverage, finished | playback only |
| `watch-tracking.json` | the **outbox**: events not yet delivered to every enabled tracker | playback and marks, only when a tracker is enabled |

Order is fixed: the record first, the outbox second. A failure to queue is a log
line, never a failed playback or a failed mark.

## The contract

```rust
pub trait WatchTracker: Send + Sync {
    fn id(&self) -> &'static str;      // "simkl"; also the credentials name
    fn name(&self) -> &'static str;
    fn deliver(&self, event: &TrackEvent) -> Result<Delivery>;
}

pub struct TrackEvent {
    pub seq: u64,                       // assigned by the outbox
    pub at: DateTime<Utc>,              // when it happened, not when it is sent
    pub title: TrackedTitle,            // imdb, kind, name, year
    pub view: TrackedView,              // Movie | Episode { season, number }
    pub watch: TrackedWatch,            // Watched | InProgress { position, duration } | Unwatched
}

pub enum Delivery {
    Delivered,
    Unmatched(String),                  // permanent: the catalogue does not know it
    Retry(String),                      // temporary: network, 429, 5xx
}
```

A tracker that cannot be used at all (a dead token, a transport that cannot send
a header) answers `Err`: the drain stops that tracker for the run and keeps the
event pending, so a re-sign-in picks it up.

## The outbox

`heronite_watch_state::tracking` owns it.

- **Folding.** The same title and episode (by IMDb id, or by name when there is
  none) still waiting is replaced by the newest state: a half-watched entry may
  not survive a later finished one. An event a tracker already consumed is never
  folded — what happened twice is two things once one of them was sent.
- **Retries.** `Retry` sets the next attempt 1, 5, then 30 minutes out (and stays
  there); the next app start drains anyway. `Unmatched` ends the event for that
  tracker and puts `"{title}: {reason}"` in `notices`, which the card shows
  **beside** the sync state: a refused title is not a sync fault, so it never
  hides "everything is sent", and signing out of the account clears it.
- **Pruning.** An event leaves the file when every *enabled* tracker is done with
  it. An event a disabled tracker still owes stays, so turning a tracker back on
  does not lose those days.
- **Bounds.** 500 events, 8 notices, owned sequence numbers never reused. Unknown
  fields are preserved, like every other Watch file.
- **Only while something is listening.** Nothing is queued when no tracker is
  signed in and enabled; the history from before is `backfill`'s job.

## Simkl

`heronite_watch_simkl::sync` builds the requests; every one carries
`client_id`, `app-name` (`hearthdeck-hero`) and `app-version` in the query, and
the person's access token as a bearer.

| Local event | Simkl |
|---|---|
| `Watched`, film | `POST /sync/history` → `movies: [{ ids: { imdb }, title, year, watched_at }]` |
| `Watched`, episode | `POST /sync/history` → `shows: [{ ids, title, year, seasons: [{ number, episodes: [{ number, watched_at }] }] }]` |
| `Unwatched` | `POST /sync/history/remove`, the same body without `watched_at` |
| `InProgress` | `POST /scrobble/stop` with `progress` (1–79), a **paused playback** |

No id is looked up before a write. Simkl resolves `imdb` (plus `title`/`year`) on
`/sync/history` **and** `/scrobble/*` server-side, and its *Mark as watched*
guide says so plainly: a `/search/id` round-trip is "wasted work that doubles your
API quota for no gain".

Simkl's history is a list of watch events and has no "40 % watched"; its paused
playback is where a resume position lives, which is why the half-watched state
goes there and below its 80 % line (nothing enters the history). Requests are sent
one at a time: Simkl holds a 20-second per-user lock and a second in-flight
scrobble collides with it (`400 RATE_LIMIT`), which the outbox treats as a retry.

Answers are read by status: `200/201` → check `not_found` (→ `Unmatched`);
`409` → already recorded (→ `Delivered`); `400 RATE_LIMIT`, `429`, `5xx` and
network faults → `Retry`; `401/403` → `Err` with "sign in again"; other `400`/`404`
→ `Unmatched`.

## When it runs

- the end of a playback (`playback::play`, after both local writes);
- after a mark by hand;
- when Simkl is signed in on the TV;
- once each time Settings → Watch → Sources is opened;
- **Sync now**, and **Send my history** (`trackers::backfill`: every finished
  session in `watch-sessions.json`, with its own timestamp).

All of them are background tasks; none of them blocks a screen or a playback.

## Screens

- The film page's header says the state; an episode row wears one of three marks
  and its percentage when it was started.
- **Mark as watched / as not watched** is a row on a film, and a row for the
  picked episode on a series — the same mark a playback would leave, and the same
  tracker event.
- Settings → Watch → Sources → **Trackers**: the `Simkl` switch, with what the
  outbox still owes and the last failure on the line under it. **Switching it on
  is the first sync** (the finished history already on the machine, with its own
  timestamps); after that only what happens is sent. The console has the same
  row.

## Not in this slice

- **Pulling** from Simkl — one way for now. The pull slice follows the guide's
  **two-phase model**, not a full pull per run:
  1. *Phase 1, once per account*: `GET /sync/activities` and keep the whole
     response in memory as the snapshot; then pull
     `/sync/all-items/{shows,movies,anime}` **sequentially** (episode-level apps
     add `extended=full&episode_watched_at=yes&include_all_episodes=yes` for the
     one-time episode baseline); only then save the snapshot, and nothing at all
     if a pull failed.
  2. *Phase 2, forever*: poll `/sync/activities`, fetch only the buckets whose
     timestamps moved, with `date_from=<snapshot>`, and move the snapshot
     forward. Never call a watchlist endpoint without reading activities first,
     and never run a polling timer without the person doing something — the
     guide names both as the way apps get throttled.
  3. Resuming across devices rides the `playback` activity: when it moved,
     `GET /sync/playback/{movies,episodes}`. Only then does a remote position
     meet the local one (newest wins), and only for a title played here.
- Real-time scrobbling (`/scrobble/start`, `/pause`): a mid-playback hook in mpv,
  so "now watching" and the 80 % auto-mark are a later slice. The paused
  playback above is the end-of-session version.
- Rewatch sessions (`?allow_rewatch=yes`, PRO-gated), ratings, memos, and
  watchlist statuses (`watching`, `dropped`, …).

## Tests

`watch-core` derives the states and the series count; `watch-state` exercises the
outbox (folding, the retry schedule, refusals, pruning, bounds); `watch-simkl`
builds and classifies every request over a fake transport; `watch` covers what a
playback and a mark queue, and the backfill; the app covers the status line and
the episode marks. No test touches the network.

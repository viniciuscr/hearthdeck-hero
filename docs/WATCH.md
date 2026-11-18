# Watch (Stremio first) — implementation contract

Status: specification for Phases 0–5. `docs/FORK.md` owns the fork's
change-control rules; this document owns the Watch feature's definitions and
the rules that apply while building it.

> **Revision 2026-11-18 — a trailer's still is wide, and never cut.** The card
> shows the trailer's picture whole: the sources offer **wide (16:9) stills, best
> first** (`Trailer::stills`, e.g. YouTube's `maxresdefault` → `hq720` →
> `mqdefault`), the loader takes the first that arrives with `Fit` (no crop) and
> the page draws it with `Contain`. The old `hqdefault` is 4:3 with black bars
> baked in, so cropping it to a banner cut the picture — that was the "broken"
> look. The details cache schema is 3, so a week-old answer with the 4:3 URL is
> fetched again.
>
> **Revision 2026-11-18 — Simkl metadata resolves through `/redirect`, and
> nothing is on by default.** `/search/id` is Simkl's *search*, and AUTH V2 puts
> search behind the account's token — a signed-out machine got **401** and the
> merge silently lost the source. The docs' own recommendation is cheaper and
> token-free: `GET /redirect?to=simkl&imdb=…` answers a `301` whose `Location` is
> the Simkl page (`simkl.com/tv/17465/…`); read the header (never follow it), take
> the id and the catalog from the path, then ask the Cloudflare-cached record
> endpoint. That needs one new transport primitive, `Transport::get_location`,
> implemented by `UreqTransport` with `max_redirects(0)`. Search is gone from the
> metadata source entirely.
>
> Also: **the sources start empty.** Catalogues, metadata and trackers are all
> `[]` on a fresh profile — the person turns on what they want in Sources, and
> some, all or none are all valid states. The one exception is the media provider
> (`stremio`), because it *is* the space: its account, its rails and its streams.
> And a manual mark now confirms itself with a toast ("Marked as watched"),
> because a row that flips silently leaves a person pressing it twice.
>
> **Revision 2026-11-18 — a trailer tab, from any metadata source.** The details
> card gained a third side: **Trailer**, offered only while a metadata source
> brought one — the page asks the merged `TitleDetails`, never a particular
> service, so TMDB or Cinemeta can supply it the day they fetch videos. The field
> is neutral (`trailers: [{ name, kind, url, thumbnail }]`); one source's list
> wins as a whole, no half-lists stitched. Simkl supplies them today: its
> `trailers` array (YouTube ids, in its own order — English first, trailers
> before teasers) becomes a page (`youtube.com/watch?v=…`) with the video's still.
> LB / RB cycle Details → Streams → Trailer; **X plays it in mpv**, the same
> player a title uses (and A does the same for a couch), leaving nothing behind —
> no progress, no session, no tracker event. mpv resolves the YouTube page through
> `yt-dlp`, which the bootstrap installs (`pacman -S yt-dlp`); pressing X again
> stops it, and a stream already playing has to stop first. The addon protocol's
> meta still reports none, and a title with no trailer simply has two tabs.
> Three things make the tab reliable: the cached details carry a **schema**
> (`details::SCHEMA`, 2 as of the trailers), so an answer written before the field
> existed is fetched again instead of hiding the tab for a week; the machine's
> own credentials (`tmdb-api-key`, `simkl-client-id`) are **adopted when details
> are asked**, not only when Settings opens, so a page is not left without the
> source just because nobody visited that page; and a metadata source that cannot
> answer now says so in the log instead of vanishing from the merge.
>
> **Revision 2026-11-18 — the settings split: Accounts and Sources.** The
> Providers page had grown into two pages' worth of questions — *"who am I signed
> in as?"* and *"what feeds the tabs and the film page?"* — so it is now two
> sections. **Accounts** is sign-in and nothing else: Stremio's link, TMDB's
> session, Simkl's sign-in, each with Sign out (and no "Forget": clearing the
> app's credential is the bootstrap's or the file's job, not a button a person
> presses by accident). Cinemeta has no account, so it has no card. **Sources**
> is four short groups, one switch per thing, named after the thing itself:
> **Streams** (`Stremio`, plus "Stop the service after watching"), **Catalogs**
> (`Cinemeta`, `TMDB`), **Metadata** and **Trackers** (`Simkl`). A metadata
> source's switch *is* its priority — one value per source, `Off` or `1`..`n`,
> changed with ◀ ▶ in console mode and a select on the desktop; the list order is
> what the details page follows, no move rows, no second "not used" list. The
> details cache keys include that order, so changing a place refetches. Switching
> a **tracker** on is its first sync: the finished history already on the machine
> goes over (with its own timestamps) and only what happens afterwards is sent;
> its line under the switch says what is waiting, or the last fault, and the
> titles Simkl could not match. The file gained `streamProviders` (default
> `["stremio"]`): off refuses that provider's streams and nothing else — its
> account and rails stay. The section answers to `sources`, `streams`, `catalogs`
> and `catalog`; the last one no longer opens Providers.
>
> **Revision 2026-11-18 — trackers: what is watched leaves the machine, after the
> fact.** Watch's history is local and stays local: `movies.json` and
> `watch-sessions.json` are written first, and a **tracker** (Simkl, the only one
> so far) is a follower told about it afterwards. `docs/WATCH-TRACKERS.md` owns
> the contract; the shape is: an event (`watched`, `in progress`, `un watched`)
> about a title or one episode goes into the **outbox** (`watch-tracking.json`)
> when a tracker is enabled, and a background drain delivers it when the network
> lets it — at the end of a playback, after a mark, at sign-in, when Settings →
> Watch → Providers opens, or from **Sync now**. The outbox folds the newest
> state of the same title/episode, retries a temporary fault on a 1/5/30-minute
> schedule, stops retrying a title Simkl cannot match (and says which), and is
> pruned once every enabled tracker has it. A tracker never blocks a playback.
>
> What a screen shows about progress gained a third state: **unwatched**,
> **in progress** (the half mark, and "In progress · 40 %") and **watched** (the
> tick), derived from the position, the length and the `finished` flag — 70 % is
> watched, one minute is the line where watching starts. A series counts its
> episodes ("3 of 8 episodes watched") instead of claiming to be watched when its
> few records are finished, and each episode record keeps its `season` and
> `number` so a tracker is never handed a provider-shaped id. A mark can be made
> by hand — on a film, and on the picked episode — writing the same record a
> playback would and queueing the same event.
>
> Simkl receives a finished film or episode as `POST /sync/history` stamped with
> **when it was watched**; an un-marked one as `/sync/history/remove`; a title
> left part-way as `POST /scrobble/stop` with its progress — a **paused
> playback**, which is Simkl's own "continue watching", because its history has
> no notion of half a film. Every request carries `client_id`, `app-name` and
> `app-version`, and the person's token as a bearer — and **no id is looked up
> first**: Simkl resolves `imdb` (with `title`/`year` as the fallback) on every
> write, so a `/search/id` round-trip would only double the quota. Requests go
> one at a time, because of Simkl's 20-second per-user lock. Simkl is also a
> **metadata source**, and only when the person asks: it joins the registry last
> and `metadataSources` does not list it, so Settings → Watch → Providers shows a
> **Describe titles** switch on its card (off until turned on; the details cache
> keys include the source set, so turning it on refetches). Catalog lookups are
> the one thing Simkl exempts from the user token, so its answer needs only the
> app's client id: the IMDb id is resolved through `/search/id`, `/movies` or
> `/tv`/`/anime` answers the card (title, year or year span, release, runtime,
> IMDb score, overview, genres, director, country, poster and fanart through
> Simkl's image proxy, certification, network, budget, original language), and a
> series adds `/tv/episodes` for the picker. It is never a catalog: no tab pages
> through it, and trailers and the rest of its catalog are a later slice. The
> pull slice is
> pinned to the guide's two-phase model in `docs/WATCH-TRACKERS.md`: one snapshot
> of `/sync/activities` before a sequential full pull, then `date_from` deltas —
> never a timer without the person doing something. **Send my history** posts
> the sessions already on the machine, with their own timestamps, so a library
> that predates the tracker appears there. One way only for now: nothing Simkl
> knows is pulled back (that is its own slice), and real-time scrobbling
> (`/start`, `/pause`, the 80 % auto-mark) needs a mid-playback hook and comes
> later with it.
>
> **Revision 2026-11-18 — Simkl is the second account, and the phone approves the
> TV.** Simkl is a **tracker**: it keeps what a person watched in step across
> devices, and it plays nothing. It therefore joins Settings → Watch → Providers
> as an **account** card (a third group, after the sources; its registry entry is
> `heronite_watch::registry::ACCOUNTS`, and it gains a proper one the day it
> serves titles). The credential is the application's — a **client id**, asked
> for once by `scripts/bootstrap.sh` into `~/.config/hearthdeck/simkl-client-id`
> (0600) and adopted into the credential store when Settings opens, or compiled
> in through `HEARTHDECK_SIMKL_CLIENT_ID` — and the person's part is the
> **device/PIN flow** of AUTH V2, the flow a TV can actually do. **One button,
> and it does the whole flow**: **Connect Simkl** asks `POST /oauth2/device` for a
> code and opens a **pairing panel** over the settings — the eight-character code
> at the top, `https://simkl.com/pin?user_code=…` as a QR under it, and the wait
> under that — so the menu carries no rows of its own for the code or the wait.
> While the panel is up the app polls `POST /oauth2/token` every interval Simkl
> returned (five seconds), adding five seconds when it answers `slow_down`, and
> **the approval closes the panel by itself**; the code's fifteen minutes end the
> wait (Simkl records no refusal, so that deadline is the only other way out).
> A network fault does not end it — the person is mid-scan — and only a refused
> client id does, with the sentence that says so. The tokens (`accessToken`,
> `refreshToken`, `scope`, `expiresAt`) are kept in `simkl-auth.json` (0600)
> beside the client id; **sign out** drops them and keeps the id, **forget**
> drops both from the credential store (a `simkl-client-id` file the bootstrap
> wrote is adopted again on the next start, exactly like TMDB's key file).
> Nothing consumes the tokens yet — the calls they unlock are the next slice — so
> a failed sign-in degrades to "not signed in" exactly like TMDB's.
>
> The same shape applies to the other two: **Stremio's link code** and **TMDB's
> approval page** also open the panel from the one button, are polled every five
> seconds, and close it when the phone approves. The old "Check" / "I approved
> it" / "Show the code as a QR" buttons and rows are gone: the poll rides out a
> network fault on its own. The panel closes with `Layer::close_with`, so a task
> that finishes later never dismisses a keyboard the person opened meanwhile.
>
> The TMDB step of `scripts/bootstrap.sh` was half-implemented and is now whole:
> the plan lists it outside `--lockdown`, and the key and the optional v4 read
> token are asked for independently — an existing `tmdb-api-key` no longer
> skipped a missing `tmdb-access-token`.
>
> **Revision 2026-11-16 — themes are presented per space, and Watch edits its
> own.** The Watch theme is no longer a frozen copy of a game preset, and a
> theme's structure is no longer rewritten on every load. Two steps, named in
> `app/src/fork/watch/space_theme.rs`:
>
> - **`validate`** runs on every load and after every change. It only *removes*
>   what the space cannot draw — a Games menu entry, a `shelf` tab or a
>   `provider:` rail in Games; `installed`, `pc`, `emulated`, `status:*`,
>   `category:*`, `smart:*` and `platform:*` tab sources in Watch — so it is
>   idempotent and can never undo an edit.
> - **`present`** runs when a theme is *taken* (chosen in the Theme row,
>   imported, reset, or a space's first run). It validates, then adds the tabs
>   and rails Watch needs (Movies, Series, Shelf, Continue watching), fitted to
>   `MAX_TABS` (`F-015`: six, because the canonical four
>   left no room for a theme's own tab). It never renames or reorders what the
>   theme brought, and only the upstream home words "All games" / "Games" are
>   read as **Watch**.
>
> Consequences, all intended: a preset's layouts reach Watch (Living room's
> **Series X** home is a Series X screen there; Classic's Library grid tab
> survives next to the theme's home tab), a fresh Watch starts from **Classic**
> with `basedOn: "classic"` (so the Theme row names it instead of reading
> "Customised"), and Customize → **Tabs**, **Shelves** and **Hero** are editable
> in Watch while **Menu** and **Game page** stay locked (their entries and
> styles are the games').
>
> The editors follow the space: a tab's source picker offers `all`,
> `type:movie`, `type:series`, `shelf`, `genre:*`, `store:*` and the catalog
> rails in Watch; the shelf picker offers Continue watching, the catalog rails,
> genres and providers; and `isEdited` compares a theme with the presentation of
> its base, so taking a preset is not an edit while a renamed tab is.
> `MAX_TABS` is six (`F-015`). A catalogue tab pages its feeds in **any**
> layout, not only the Grid, or a Series X "Movies" tab would draw nothing.
>
> The hero speaks watch: **facts** are the provider it streams from ("Stremio"
> today, "Netflix" or "YouTube" when those providers exist) and the year — never
> "Installed" or a playtime — and **stats** are the watched time, when it was
> last watched and the genre. Both come from the library's own records
> (`last_played`, `playtime_minutes` from Stremio's timestamps, release date and
> genres from the metadata projection): the home screen asks no metadata source
> anything, so runtime and the IMDb score stay on the film page, which already
> loads `TitleDetails`.
>
> **Backups carry Watch** (2026-11-16): a backup's rule is *everything that is not
> left out is in*, so Watch's files were already inside `Part::Settings` — the
> history (`movies.json`), the session log (`watch-sessions.json`), the shelves
> (`watch-personal.json`), the settings and the Watch console theme. That is now
> pinned by name in the catalog's tests instead of riding the default. The one
> change is the credentials: `stremio-auth.json`, `tmdb-auth.json` (and the next
> provider's) are **`Part::Logins`**, so a backup can exclude them and the cloud
> sync — which uploads only `Collection` and `Settings` — never sends an account
> key anywhere. A restore without them keeps every record: progress, sessions and
> shelf snapshots are keyed by provider and id, so they come back the moment the
> account is linked again.
>
> **Watched is local, per episode** (2026-11-16): a title's record
> (`movies.json`) keeps **one state per episode finish** — `episodes: { videoId →
> { positionMs, durationMs, updatedAt, finished } }` — beside the resume pointer
> (`video_id`), which only says where playback starts. Before this the page
> ticked the *pointer's* episode whether or not it had been finished, so opening
> the next episode un-ticked the one just watched and an unfinished episode
> looked complete; those were the same bug. Now a tick means finished, a row says
> "Resume from here" when it is the pointer and unfinished, a film says
> **Watched** only once its own record crossed the threshold, and finishing an
> episode never un-watches the one before it. The IMDb id (`tt…`, the id Watch
> already resolves titles by) is stored **with** the record (`imdb`), so a record
> survives a provider that renames its own ids. Stremio's remote write is
> unchanged and still best effort: the local file is the truth, and another
> provider can sync to it later. Stremio's own per-episode `watched` bitfield
> remains unread — our ticks no longer depend on it, which is why they are
> consistent.
>
> **A recap, from a session log** (2026-11-16): every playback appends one record
> to `watch-sessions.json` (`crates/watch-state`): provider, kind, title, episode,
> start and end, the time that session *covered* (the distance the position
> moved — resuming at 40 min and stopping at 55 is fifteen minutes), the length
> and whether it reached the finished threshold. Under 45 s is not recorded; a
> title keeps its newest 2000; the file is never read to play anything, so it can
> be deleted without losing a position. `heronite-watch-stats` turns the log into
> a window — week, month, year, everything — of day buckets, titles, totals,
> streaks, binge days and the early/evening split, and Settings → Watch → **Recap**
> draws it in Settings, and `☰ → Statistics` (the sidebar's Statistics on the
> desktop) is **space-aware**: in the Watch space it is the recap, drawn by
> `app/src/fork/watch/stats_page.rs` with LB / RB switching the window, and the
> games' page in the Games space. The Watch space's menu keeps the `statistics`
> item for exactly that reason — `space_theme::validate` used to drop it. History
> before this shipped exists only as Stremio's lifetime `totalPlayed`, which the
> recap deliberately does not fold in.
>
> **Settings are grouped by provider** (2026-11-16): **Watch** (the theme and its
> reset) / **Providers** (a card per registered provider, streaming services
> first, then the sources: name, what it gives, its account or key and status,
> and the one switch that turns it on — generated from the registries, so a new
> provider needs no Settings code) / **Home** (every rail the first page draws,
> each labelled with its provider and toggled there). `settings.homeRails` is the
> list and the drawing order; it holds `continue:<provider>` for a media
> provider's Continue watching and collection ids for the rest, and it is what
> `collections` was called while the rails lived on the Shelf (the old key still
> reads). In Watch the first page is **only** that list: a theme brings no
> shelves, `validate` empties them and the Shelves card is locked again. Deferred:
> a Defaults page choosing which provider owns a new watchlist/favourite entry,
> until more than one provider can own a list.
>
> The desktop theme editor only ever writes the **Games** prefs; the Watch theme
> is edited from the Watch space (Customize, or Settings → Console mode while
> Watch is on screen). A `shelf` tab cannot reach Games any more: `validate`
> drops it, and the shelf source draws nothing outside the Watch space.
>
> Signing in to TMDB exists as of this revision, and **asks for no password**.
> The API key stays the application's credential — every call, the sign-in
> included, is authenticated with it — and a **session** is the person's:
> Settings → Watch → Catalogs has **one** entry point, "Connect TMDB": it reads
> the API key **from the clipboard** (a 32-character key is not typed, least of
> all with a controller) and then asks TMDB for a request token, opens
> `themoviedb.org/authenticate/{token}` in the person's own browser (through
> `open_url`, so luakit, Firefox or whatever they use), and exchanges the
> approved token for a session id, kept beside the key in the same `tmdb`
> credential (`sessionId`, `username`). There are no separate "paste / type /
> remove the key" rows any more: once connected, the section shows
> "Signed in as …", "Sign out" (which keeps the key) and "Forget TMDB" (which
> clears both). While the approval is pending the same link is drawn as a **QR
> code**, so a phone approves it without the host's browser; console mode opens
> that code as an overlay (the same element the desktop draws). TMDB reports the
> approval only through step 3 (`/authentication/session/new`), and a phone is a
> different device — a `redirect_to=127.0.0.1` would land on the *phone's*
> loopback — so the app **asks step 3 again every five seconds** for ten minutes:
> scan, log in, approve, and the card turns into "Signed in as …" on its own.
> "I approved it" stays as the instant path. A 401 on the exchange means "not
> approved yet", not a bad key: the key was accepted a moment earlier. Nothing
> is used by the first rails of the person's own: `/account/{id}/{movie,tv}/
> recommendations` are offered in Settings → Watch → Collections **only while a
> session exists** (`CatalogRequest::account` routes them, `rails::title` names
> them "Movies · Recommended for you"), drawn on the first page like any other
> collection. Writing to the account (watchlist, favourites, rated — §9 of
> `docs/WATCH-COLLECTIONS.md`) is still deferred, and signing out keeps the key,
> so the catalogs keep working.

> **Revision 2026-11-15 — metadata sources you choose, merged field by field.**
> What describes a title is a **metadata source**, and which ones answer is the
> person's choice: `metadataSources` in `watch-settings.json` (Settings → Watch →
> Metadata providers), in priority order, with **TMDB first and Cinemeta as the
> fallback** — nothing is built around Cinemeta being the one that answers, and it
> can be turned off. The sources are asked in that order and the answers are
> **merged**: the first to fill a field keeps it, a later one fills only what is
> still empty, and the open facts (`extras`: a tagline, a budget, a network,
> where to watch) are joined with the first of each id winning. `TitleDetails`
> therefore renders as "everything the chosen sources know": a source that knows
> more adds rows to the card, one that knows less cannot take away what another
> gave. The cache remembers the set that answered, so toggling a source never
> serves the old merge for a week.
>
> **Revision 2026-11-14 — catalog sources, and TMDB as the second one.**
> Browsing is its own contract. A **catalog source** (`WatchCatalogSource`)
> declares the catalogs the Movies and Series tabs may page through — a name, a
> kind, the orders it can serve, its filters, whether it takes a search and
> `skip` — and answers a page; it plays nothing, owns no library and is never
> asked for a stream, so `WatchProvider` no longer carries catalog methods.
> Cinemeta became a catalog source of its own (`heronite-watch-catalogs`) and
> TMDB (`heronite-watch-tmdb`) is the second: `discover` for films and series
> with TMDB's real sorts and genres, every result resolved to its IMDb id
> through `/external_ids`, and a title TMDB holds no IMDb id for is left out
> because it could not be played. Settings → Watch → Catalogs turns sources on
> and off and keeps the TMDB API key (credential store, 0600); a catalog decides
> what is listed and never what plays. TMDB's own "watch providers" (Netflix, …)
> are availability, not our streams: they belong to the metadata layer.
>
> **Revision 2026-11-14 — a film page of its own, and metadata sources.**
> Films and series get a page built for them (`app/src/collection/console/
> watch_page.rs`) instead of the games' page: provider, year, runtime, rating,
> what was watched, the description and the cast and crew. What describes a
> title is a **metadata source** (Cinemeta today), a different contract, crate
> and registry from the **media provider** (Stremio: titles, streams, progress);
> a provider is never asked to describe a title, and `details()` is not on
> `WatchProvider`. A series reports its status, seasons and episodes, and the
> film page carries the picker: a season row moved with ◀ ▶ and one row per
> episode, with the card following the focused episode. The card has two tabs,
> LB / RB: **Details** (what the film or the picked episode is) and **Streams**
> (where it can be played from), for films and series alike — the streams are no
> longer rows of the list. ◀ ▶ move the focus between the list and the card; the
> side holding it takes ↑ ↓ and A, and the details card scrolls with ↑ ↓ when it
> has the focus (the right stick scrolls it either way). A on an episode, and the
> Play button, start the best stream at once. Watched marks follow the stored episode pointer (`video_id`);
> Stremio's per-episode `watched` bitfield is still out. HowLongToBeat is no
> longer asked about titles in the Watch space.

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

> **Revision 2026-11-11 — Watch settings and migration** (superseded by
> 2026-11-16: the backup and the one-time replacement are gone — `validate`
> turns a game-shaped file into a watch theme in place, and the theme is
> presented from Classic, not copied into `app/src/console/watch_theme.rs`).
> Settings → Watch owns the Stremio account, its rails and the collections; console
> Customize edits the same Watch theme. A game-shaped `console-watch.json` or
> `watch-ui-state.json` was backed up as `*.json.bak` and replaced once with the
> Watch defaults: Home / Movies / Series, and Continue watching / Top movies /
> Top series.

## Scope

**In:** Linux/CachyOS, films and series, a Stremio account linked by code,
discovery through Cinemeta, a film page fed by metadata sources (Cinemeta's
`meta` resource), streams from the account's `stream` addons (Torrentio
is the expected one), playback through the installed Stremio Service and a
supervised `mpv`, and watch-progress read/write through Stremio's datastore.

**Out (v1):** per-episode watched state from Stremio's `watched` bitfield,
season 0 (the specials) in the picker, Windows playback, an in-app video
renderer, any plugin/addon host, and any change to the game catalog, its data
files or its launch paths.

**Non-negotiable:** a gamepad must control every playback path we ship, either
directly or through a documented handoff. No secret may reach a log, an error
string or a crash report. No write may delete or un-watch an account record.

## Boundaries

| Unit | Responsibility | May depend on | Must not |
|---|---|---|---|
| `crates/watch` (`heronite-watch`) | composition root: provider registry, browse → proxy `Library`, playback orchestration, the app's import surface | `crates/watch-{core,state,player,stremio}`, `heronite-data` | GPUI, provider protocol code |
| `crates/watch-core` | domain types, `WatchProvider`, the `Transport` and `Credentials` ports | serde, url, chrono, ureq | `heronite-*`, files, processes |
| `crates/watch-addon` | the Stremio addon protocol: manifest, catalog, meta, stream | `crates/watch-core` | accounts, datastore, Cinemeta |
| `crates/watch-player` | mpv discovery, arguments, IPC supervision | `crates/watch-core`, `heronite-data` | protocol and provider types |
| `crates/watch-state` | local files and the Heroic-format projection | `crates/watch-core`, `heronite-data` | network, providers |
| `crates/watch-stremio` | the Stremio provider | `crates/watch-{core,addon}` | files, processes, `heronite-data` |
| `app/src/collection/` (`Space::Watch`) | The same Collection/console screens, loading the proxy `Library` | `crates/watch`, `heronite-data` | A second grid, Focus panel or shelves implementation |
| `app/src/collection/nav.rs`, `collection/console/*`, `collection/palette/*` | Navigation and shell route entries | — | Feature logic |

`docs/WATCH-CRATES.md` owns the crate map, the provider contract and the rules
for adding a provider.

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
    pub api: Url,                 // https://api.strem.io/api/
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
- A series asks per episode, with the episode's own video id as the id:
  `stream/series/{tt-id}:{season}:{episode}.json`. Its `meta` `videos[]` names
  those ids (`tt0944947:1:1`); one without an id is named that way here.
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
- Health: probe `/settings` before playback. When the service is down the app
  starts it and waits (8 s per attempt) for it to answer, trying the headless
  `stremio-runtime <server.js>` first and the `stremio-service` launcher after
  it: the launcher builds a tray icon, so it exits at once without a desktop
  session. Both are found through `STREMIO_SERVICE`, then `PATH`, then
  `/usr/share/stremio-service` and `/opt/stremio`. A launch that exits or never
  answers contributes its last error lines to the report.
- The service's `baseUrl` names the machine's **LAN** address (for other
  devices); the player always fetches from loopback, keeping only the answered
  port, so the stream stays inside the loopback trust boundary.
- The app owns only the process it started: Settings → Watch decides whether it
  is stopped when the session ends (default on, to free its memory); a service
  that was already running belongs to the person's desktop session and is never
  touched.
- Never assume "localhost" is trusted: the service is a separate process, not
  our code.

### 3a. Browse-all catalogs (probed 2026-11-14)

- A **catalog source** declares the catalogs the Movies and Series tabs browse
  (`WatchCatalogSource`): each with a name, its kind, the orders it can page by
  (a closed set: Popular, Rating, Newest, Title), its filters (id, label,
  values, required) and whether it takes a search and `skip`. The panel draws
  what the enabled sources declare; with none enabled the tabs keep what the
  rails carry. `WatchProvider` (titles, streams, playback) carries no catalog
  method at all, so a catalogue that plays nothing cannot pretend to be one.
- Cinemeta declares three per kind: `top` ("Popular"; filter `genre`; search;
  paging), `imdbRating` ("Featured"; order Rating; filter `genre`; paging) and
  `year` ("New"; order Newest; a **required** filter whose values are years and
  which travels in the protocol's `genre` slot).
- TMDB declares one `discover` catalog per kind: sorts Popular
  (`popularity.desc`), Rating (`vote_average.desc`, with a vote floor), Newest
  (`primary_release_date.desc` / `first_air_date.desc`) and Title
  (`original_title.asc` / `original_name.asc`), its genres from
  `/genre/{movie,tv}/list` (remembered for the process) and a year; `search` uses
  `/search/{movie,tv}`, and paging is TMDB's own `page` (twenty a page, so `skip`
  becomes a page number). The API key is read from the credential store under
  `tmdb` and sent as `api_key` (the transport carries no headers); without one
  the source says where to put it. Its chip is called **TMDB**: `discover` is the
  endpoint, not the catalogue.
- TMDB answers TMDB ids, and the rest of Watch keys everything by IMDb id, so
  each discover result costs one `/external_ids`. Those answers never change, so
  they are kept under `<cache>/watch-catalog/ids/` (delete-safe), and the misses
  of a page go out four at a time instead of twenty in a row; a title TMDB has no
  IMDb id for is remembered as such and never asked again.
- Probed with redirects followed — every catalog answers **307** to
  `cinemeta-catalogs.strem.io/<id>/catalog/…`, which the transport follows:
  `top` 50; `imdbRating` 49, then 50 at `skip=50`; `year/genre=1999` 50, and 50
  more at `skip=50`; `year/genre=2008` for series 50; `imdbRating/genre=Sci-Fi`
  49; `top/search=matrix` 21. `skip` is honoured past 400 on `top`. `search`
  answers even where the manifest does not declare it, but the tabs offer it only
  where the provider advertises it.
- Extras are **path segments** (`catalog/{type}/{id}/skip=100&genre=Sci-Fi.json`),
  percent-encoded; a query string is ignored by both hosts.
- A page's items carry the feed that produced them (`watchCatalog` = source,
  kind, catalog, order and filters), and a page counts how many of the source's
  own results it consumed — TMDB drops the titles it has no IMDb id for, and its
  pages are a fixed twenty, so `skip` advances by what was consumed and the next
  page is asked for rather than the same one again. Changing a chip, the order or a filter
  **replaces that kind's pages** in one rewrite and starts again at `skip=0`;
  scrolling appends. The rails (no feed) and the other kind are untouched.
- A tab lists its feed's own ids once it has asked for a page, so the rails never
  leak into a catalog and a search shows its own titles.
- **Browsing never lives in a media provider.** Stremio's `browse` reports only
  what its own account knows — where the person stopped — and reaches no
  catalogue at all. The **first page** draws the provider collections Settings
  lists (one rail each, in that order), so turning a collection off removes it
  from the page and turning a source off removes what it contributed from the
  first page *and* from the tabs.
- The Movies and Series tabs list only their own feed's titles: before the first
  page arrives they are empty, with a message that says why (asking the sources,
  none turned on, or nothing found). They never show the library and replace it a
  moment later.
- The app asks for the next page when the grid's last visible row is within two
  rows of the end (the wheel and the pad both move that), shows "Loading more…"
  while a page is in flight, and retries a failed page only after 20 s — a fetch
  per frame is what made the grid blink. A page that lands after the feed changed
  is dropped (the feed's generation).
- A catalog tab is ordered by what the provider sends, which is why the order row
  appears only where a catalog offers more than one.

### 4a. Stream resolution

- Read the session key, then `addonCollectionGet`; keep the addons whose
  manifest serves `stream` and whose `types` include the title's kind.
- Both live at `https://api.strem.io/api/`: the path is part of the base URL,
  not something `Url::join` adds (`https://api.strem.io/addonCollectionGet` is
  a 404).
- `addonCollectionGet` answers `{"result":{"addons":[…]}}`; the decoder also
  accepts a bare `result` array for older answers.
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
- A series' record stays keyed by the series id and carries the episode in
  `video_id` (the video id the addons used); resume applies only when the stored
  pointer names the episode being played, and another episode starts at zero.
- Local `movies.json` is the source of truth between writes; Stremio is the
  cross-device source. The picker opens on the stored pointer and ticks it.

### 7. Metadata sources (verified 2026-11-15)

- What **describes** a title is a metadata source, not a media provider. The
  media provider (`WatchProvider`: Stremio, YouTube) owns titles, streams and
  progress; a metadata source (`WatchMetadataSource`) owns what a title *is*.
  They are different traits, crates (`heronite-watch-stremio` vs
  `heronite-watch-metadata`), registries and settings. A media provider is never
  asked to describe a title.
- The answer is `TitleDetails`: name, year (`releaseInfo`), released date,
  runtime, rating, description, genres, directors, writers, cast, countries,
  awards, a series' status, its season and episode counts, and art — plus
  `extras`, the open facts (`Fact { id, label, value, source }`) a source knows
  and the page shows as they come, which is how a source that knows more changes
  the card without the page changing.
- Which sources answer is the settings' `metadataSources`, in priority order
  (TMDB first, Cinemeta as the fallback). The registry asks them in that order
  and merges: the first answer for a field wins, the next fills what is still
  empty, extras are joined with the first of each id winning. An id this build
  does not have is skipped, so the order is already right when a source lands.
- Cinemeta answers through the addon protocol: `GET {transport}/meta/{type}/{id}.json`;
  TMDB answers through its own v3 API. Both arrive through the same
  `WatchMetadataSource`.
  Cinemeta is a source over its manifest
  (`https://v3-cinemeta.strem.io/manifest.json`), and TMDB
  (`heronite-watch-tmdb`) is the second: it starts at
  `/find/{imdb}?external_source=imdb_id` — our keys are IMDb ids and TMDB's are
  not — then reads `/movie/{id}` or `/tv/{id}` with
  `append_to_response=credits,external_ids`, and for a series one
  `/tv/{id}/season/{n}` per season so the episode picker works for titles only
  TMDB knows. Episodes keep the id the `stream` resource is asked for
  (`tt0944947:1:1`), so playback works whichever source described the series. It
  also fills the open facts: a tagline, a budget, the network, the original
  language. Its availability data ("on Netflix") is a fact when it lands, never a
  stream source. A source that needs a key reads its own entry through
  `MetaCtx`'s credentials; Cinemeta is given none.
- A 404 is "this source does not know the title" (`None`), not a fault: the next
  source is asked. A timeout, a 5xx or a body that is not JSON is an error; the
  registry keeps it aside, asks the next source, and when none answers the page
  says the details are unavailable.
- `meta` sends `null` for what it does not know, lists included. `videos` holds
  a series' episodes: season 0 is the specials, counted neither as a season nor
  as an episode.
- The merged answer is cached per `(provider, kind, id)` **and the set of sources
  that answered** under `<cache>/watch-details/**`
  (`heronite-watch-state::details`), fresh for seven days; a stale answer is
  served when no source can be reached. The files are disposable: delete them and
  they are fetched again.
- The film page shows the source's facts and none of the games' rows (installed
  state, playtime, status, folders, HowLongToBeat). Play and its stream rows stay
  the provider's.

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

1. **Compose, don't patch.** New behavior in the watch crates
   (`docs/WATCH-CRATES.md`); no edits to upstream crates; only the registered
   `app` seams.
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
| F-010 | `app/src/collection/{actions,game_actions,render_focus,grid_drawer}.rs`, `app/src/collection/focus/mod.rs`, `app/src/collection/console/{mod.rs,game/mod.rs,layouts/spotlight/mod.rs}` | Native Watch playback: the stream list and Play action on both detail surfaces | active |
| F-011 | `docs/ARCHITECTURE.md` | the watch crate map (the fork-owned watch crates are absent upstream) | active |
| F-012 | `app/src/collection/console/{mod.rs,game/mod.rs,game/hero.rs}`, `app/src/collection/hltb_ui.rs` | a fork-owned film page for Watch titles (`console/watch_page.rs`) and no HowLongToBeat lookups for them | active |

Fork-owned code (`crates/watch*`,
`app/src/collection/console/watch_page.rs`, `app/src/collection/watch_playback.rs`,
`app/src/collection/watch_service.rs`, `app/src/fork/watch/**`, this document)
is not a deviation and is not listed here; only upstream files are.

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
- **P1:** the watch crates compile on Linux and Windows; fixture tests pass; no UI.
- **P2:** link + datastore + merge/write with mock-server tests; real-account
  test ignored.
- **P3:** `Space::Watch` in the Collection (desktop), proxy Library, locales; gate green.
- **P4:** console Watch item, same shelves and game page, inline stream rows.
- **P5:** native mpv through the Stremio Service, gamepad modes; hardware
  checklist green.

## Probe log

Paths in older entries name the pre-split layout; `docs/WATCH-CRATES.md` has the
current crate map.

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
- **2026-11-13, API base probe.** `POST https://api.strem.io/addonCollectionGet`
  answers 404; `POST https://api.strem.io/api/addonCollectionGet` answers with
  Stremio's JSON envelope (an invalid key gives `Session does not exist`).
  `Endpoints::default().api` now ends in `/api/`, which also fixes the library
  sync that had been logging 404 and keeping the last good library.
- **2026-11-13, addon collection shape.** A real account answers
  `{"result":{"addons":[…]}}`; the first decoder expected `result` itself to be
  the array and failed with "invalid type: map, expected a sequence". The
  decoder now reads the `addons` key and keeps the bare-array fallback.
- **2026-11-13, null manifest lists.** Some installed addons serialize
  `resources`, `types`, `catalogs` or `idPrefixes` as `null`; `serde(default)`
  does not cover that. Manifests now read `null` as an empty list (and a
  `null` addon list as no addons); an addon with no `transportUrl` is skipped.
- **2026-11-14, Cinemeta catalogs.** `top` answers 50 items per page and honors
  `skip` in the **path** (`catalog/movie/top/skip=100.json`); the query form and
  the ordered form `?skip=300` are ignored. `genre=…` filters, `search=…` answers
  one page of 20, `movie/year/genre=1999.json` and `movie/imdbRating.json` exist
  as further catalogs. Requests redirect (307) to `cinemeta-catalogs.strem.io`,
  which the transport follows.
- **2026-11-14, the Stremio service, headless.** `stremio-service` (the packaged
  launcher) exits at once without a desktop session — `Failed to build tray
  icon: Read-only file system` — while `/usr/share/stremio-service/stremio-runtime
  /usr/share/stremio-service/server.js` answers `/settings` in about a second
  and keeps running; it finds the package's ffmpeg next to itself (the launcher
  passes `FFMPEG_BIN`/`FFPROBE_BIN`, which we now do too). `GET /settings`
  answers `baseUrl` as the machine's **LAN** address (`http://192.168.178.100:11470`
  here), which the old loopback-only check refused: the player now keeps the
  answered port on loopback instead.
- **2026-11-14, Cinemeta meta.** `GET
  https://v3-cinemeta.strem.io/meta/{movie|series}/{tt-id}.json` answers
  `{"meta":{…}}` with `awards`, `cast`, `country` (comma-joined), `description`,
  `director` and `writer` (arrays; `null` for a series' director), `genre` and
  `genres`, `imdbRating` (a string), `released` (an ISO timestamp), `runtime`
  ("136 min"), `status` (a series, "Ended"), `videos` (a series' episodes: 67
  for Breaking Bad, season 0 holding the specials), `poster`, `background`,
  `logo` and `trailerStreams`. An unknown id answers 307 to
  `cinemeta-live.strem.io` and then 404, so "no answer" has to stay distinct
  from a fault: `AddonClient::details` maps 404 to `None` and any other status
  to `HttpStatus`. The film page's facts come from `TitleDetails`; the cache
  keeps an answer for a week.



# Watch titles — the transition plan

Status: **implemented** (2026-11-19). **P1–P5, P6a and P7 are done**; only P6b —
the `PlayCopy` branch — remains, and it is gated on a second stream provider
shipping. Everything from the model to the file projection speaks `Title`, a
title played by two providers is one card with both sources recorded, and the
gates and the register review are clean. This revision also records what the two reviews of the
first draft changed (§11).

Companion to `docs/WATCH.md` (the space), `docs/WATCH-IMPORT.md` (sync) and
`docs/WATCH-CRATES.md` (the crate boundary). `docs/FORK.md` owns the deviation
register this plan feeds.

## 1. Why

Watch began inside a collection whose unit is **a game from a store**:
`CollectionGame` with `runner`, `app_name`, `source_label`, `installed`,
`family`, inside a `Library` whose `copies` group one game owned on several
stores. Watch's data had to travel in `extra` (`watchId`, `stremioType`,
`watchCatalog`) and `installed` had to be repurposed to mean "a stream source
can play this".

That is the smell this transition removes. The structure stays — a library,
rails, tiles, progress, focus, themes — only the nouns and the model become
Watch's: **stream providers** and **movies/series**.

The binding constraint is upstream. Every line we add in a shared file is a
conflict to reconcile at every merge, so the plan keeps that surface to an
explicit, registered budget (§2), machine-checked in `app/src/fork/seams.rs`.
One request is made upstream, deliberately: a generic extension point for the
console's detail page, offered as a branch cut from `upstream/main` under the
fork's upstream-first rule (`docs/WATCH-PAGE.md`). The fork never depends on it.

## 2. Ours and upstream's

| Ours (fork-owned) | Upstream's (shared) |
|---|---|
| `crates/watch-core`, `crates/watch-state`, `crates/watch`, `crates/watch-*` | `app/src/collection/{vm,data,card,tile,carousel,list_row}.rs` |
| `app/src/fork/watch/**` | `app/src/collection/console/{game,layouts}/**` (the renderer) |
| `app/src/collection/console/watch_page/**` (the film page: host, engine, sections, styles) | `crates/data/**` (`CollectionGame`, `Library`, `copies`, `json_store`) |
| `crates/watch-state/src/collection.rs` (the **watch library writer**) | the theme system, widgets, navigation, image loader |

**The rule this document exists to fix:** *shared files never learn a Watch
noun.* A Watch concept is adapted at the boundary into the shapes upstream
already consumes.

### The upstream budget

Every seam this transition touches in upstream's files. The authoritative file
list is the seam table in `app/src/fork/seams.rs`; the rows of `docs/FORK.md` say
why each exists, and the footprint below is measured in seam sites, not in lines.
The page's own seams (`F-012`) are listed in that table and in
`docs/WATCH-PAGE.md`; the rows here are the projection's.

| Seam | Files | Footprint | Why it cannot be ours |
|---|---|---|---|
| the placeholder flag (`F-019`) | `app/src/collection/{vm,data,card,list_row}.rs`, `console/tile.rs`, `console/layouts/carousel.rs`, `console/layouts/spotlight/row.rs` | one `CardVm` field (+4 lines with its test fixture), one assignment, **five** one-line grey conditions — seven files | the card model and the grey rule are upstream's renderer; "a placeholder is always dimmed, whatever the theme says" has no other expression |
| a provider's display name (`F-007`) | `crates/data/src/library.rs` (`source_label`) | **one line per stream provider** ("Stremio" is there) — an English literal the i18n coverage gate cannot see | `source_label` is upstream's table; an unknown source falls back to "Local" |
| "Play with \<provider\>" (`F-020`) | `app/src/collection/copies.rs` → `GameAction::PlayCopy` | one branch, when a second provider ships | `PlayCopy` ends in the store client's launch (`app/src/collection/game_actions.rs:424`), which cannot start a Watch stream |
| the module rename (`F-006`, `F-008`) | `app/src/collection/{load,mod}.rs`, `console/sources.rs` | one line each for the rename, plus two lines in the watchlist rail (`game.runner == item.provider` dropped, so a shelf entry matches its title on any card) | the app dispatches through that module, and the rail is upstream's; both files are already registered seams |

P5 **reduced** the first seam: the duplicated helper is gone, and `data.rs` is
now one line that calls `sources::imported` (+19/−6 became +12/−6). The second
cannot shrink below one line per provider. The third is deferred — confirmed
necessary, because `PlayCopy` never reaches Watch today.

Each is a fork-side patch with a `Fork-Seam` trailer and a `docs/FORK.md` row.
`app/src/fork/seams.rs` lists those files and the lines the fork owns, and fails
the build when the table, the tree and the register disagree. The budget is
measured in **seam sites per file**, not in lines: lines drift with upstream's
formatting, sites do not.

## 3. The model

`crates/watch-core/src/title.rs`, as it will be after P1. **No serde**: nothing
persists a `Title`; the one persisted item shape is `ShelfTitle` (§3.1).

```rust
pub enum TitleKind { Movie, Series }        // "show"/"tv" map to Series
pub struct ProviderId(String);              // a stream provider: Stremio today

/// The id space a title is known in: IMDb, or the provider's own.
pub enum IdSpace { Imdb, Provider(ProviderId) }
pub struct TitleId { pub space: IdSpace, pub id: String }
pub struct TitleKey { pub kind: TitleKind, pub id: TitleId }
pub struct Art { poster, background, logo }

/// Where a title was listed. The first mark names it.
pub enum RailMark {
    Feed { source: String, catalog: String }, // a catalogue page ("tmdb", "movie:top:Popular")
    Collection(String),                       // a rail the person turned on
}

pub struct Watched {
    pub at: Option<DateTime<Utc>>,
    pub minutes: f64,
    /// `timeOffset` / `duration` / `flaggedWatched`: what says "in progress"
    /// when `timeWatched` counted nothing (a film that was only started).
    pub position_ms: u64,
    pub duration_ms: u64,
    pub finished: bool,
}

pub struct Title {
    pub key: TitleKey,
    pub name: String,
    pub release: Option<String>,   // as the library prints it ("1999", "2011–2019")
    /// The day it came out (`1999-03-31`), when a source says it.
    pub released: Option<String>,
    /// How long it runs, in the source's words (`136 min`).
    pub runtime: Option<String>,
    pub art: Art,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub rails: Vec<RailMark>,
    /// A wire placement, not identity: Cinemeta's Top membership. Kept as a
    /// field because one persisted mark cannot carry it (§5).
    pub top: bool,
    /// The stream providers that can play it. Empty: a placeholder.
    pub providers: BTreeSet<ProviderId>,
    pub watched: Option<Watched>,
}
```

- **Identity is `(kind, id space, id)`.** A stream provider is a *source*, never
  identity. The id space is part of the key because the second provider is
  YouTube, whose titles have video ids and often no `tt` id at all — the code
  already assumes this (`app/src/collection/watch_playback.rs:680` keeps `imdb`
  only for `tt…`).
  Matching a Stremio title to a YouTube one is a **lookup** (name + release),
  recorded by the adapter, not a merge of identities; the day that matters,
  `Title` grows `alternates: BTreeMap<IdSpace, String>`.
- **`available()` is derived**, never stored: "a playable provider exists". The
  word is the product's own ("Not currently present in any available stream
  source"), but a field would let the two disagree.
- **A placeholder is a `Title` with no provider.** No flag is repurposed, no
  set is split. "Who told us" is not on the title: it is the rail mark for a
  listing (`RailMark::Feed::source`) and the personal ledger or
  `watch-imported.json` for an import.
- **`Title::merge` is the only way two observations become one**, one field at a
  time — the `TitleDetails::merge` rule:
  - name, release, released, runtime, description, genres, art: the **first
    non-empty** value wins, so a provider that answers with a poster fills a
    title another left bare; `merge` is therefore idempotent but **not**
    commutative.
  - `providers` and `rails`: union, order preserved (`rails` keeps the first
    mark as the one that names the title).
  - `watched`: the later `at` wins, with its `minutes`, its position and its
    flag; a tie keeps the left.
  - `top`: `any`.
- **Episodes are not a kind.** Catalogue pages carry `movie|series`; an episode
  is a `metadata::Episode` inside a series, with per-episode state in
  `movies.json`. `TitleKind` stays two, because `type:movie`/`type:series` tab
  sources, `stremioType` and the writer's keys all mean the two.
- **The catalogue's facts travel with the title.** `released` (the day, not the
  year) and `runtime` are what a card needs: "New" measures the first — a
  catalogue has no added date, so the library's first load would mark every
  title new — and the facts line prints the second under its own name (**Runtime**
  for a film, **Episode runtime** for a show, whose sources never give a whole
  run), because How Long To Beat knows games. Both come from a catalogue preview
  and are written into the projection's `extra`; the film page still asks the
  metadata sources for its own `TitleDetails` when it opens, and the episode
  count for **Time to binge** is theirs alone.
- **A catalogue source is not a stream provider.** A catalogue page lists
  titles; the providers that can resolve a stream for a title's id play it —
  today Stremio alone, which is why a Cinemeta or TMDB item is playable by
  Stremio and a YouTube track would not be. `RailMark::Feed::source` names the
  catalogue; `providers` names who plays.
- **`ProviderId` has two doors.** `new` validates wire input (a blank id is
  refused); `declared` takes a crate's own constant (`Stremio`'s `ID`, a
  source's `PLAYS_WITH`) and is debug-asserted non-empty. A typo in a constant
  is a bug, not a runtime state — and the registry stays the authority for which
  ids exist.

### 3.1 The one persisted shape

`watch-personal.json` is user data a downgrade must still read, so the shelf
keeps a frozen snapshot — `ShelfTitle` in `watch-core::personal`: kind, id,
name, release, art, description, genres, and the provider as *provenance*. It is
a remembered card, not a library record: carrying `providers` or `rails` would
only store what the next refresh is about to correct. Its wire names are the old
ones — `year`, and the art flat at the top level — so a file written before the
transition still loads, and a downgrade still reads what this build writes.
`watch-imported.json`'s `Placeholder` is its own shape and does not change at
all. `MovieItem` survives only as the projection's wire record, between
`record_of` and `title_of_game`.

## 4. Vocabulary map

| Today (games' words) | Watch's word | Where |
|---|---|---|
| `runner` | `Title::providers` (`ProviderId`) | domain |
| `app_name` (`{owner}:{id}`) | the projection's key, **derived**; never stored on a title | writer |
| `installed` | "a playable provider exists" (`Title::available()`); `is_installed` in the projection | domain |
| `source_label` ("Steam", "GOG") | the provider's name ("Stremio") | screens (upstream's table, `F-007`) |
| `extra.watchId` | `TitleKey::id` | domain |
| `extra.stremioType` | `TitleKey::kind` | domain |
| `extra.watchCatalog` | `Title::rails` | domain |
| `extra.watchReleased` / `extra.watchRuntime` | `Title::released` / `Title::runtime` | domain |
| `extra.watchPosition` / `watchDuration` / `watchFinished` | `Watched::state()` (`timeOffset`, `duration`, `flaggedWatched`) | domain |
| `extra.watchSource` | the first rail's source (`RailMark::Feed::source`): the catalogue that listed the title | domain |
| `extra.watchStreams` | the stream probe (`watch-state::streams`): `true` a stream, `false` none, absent never asked | domain |
| `extra.stremioTop` | `Title::top` | domain |
| `extra.imported` | `Title::providers` is empty | domain |
| `MovieItem.addon` ("cinemeta") | `RailMark::Feed::source` | domain |
| store / game | stream provider / movie, series | everywhere |

Two rows need care. `app_name` is not a title field: it is `{owner}:{id}`, the
key the projection and `CollectionGame` use, and it stays derived exactly as
today. `installed` is not renamed for games — the repurpose is confined to the
watch library writer, which is ours.

## 5. What upstream sees (the projection)

`crates/watch-state/src/collection.rs` projects titles into the Heroic shapes
`heronite-data` reads, under `<cache>/watch-library`, regenerated on every load
and never merged with the games' library.

**The keys are frozen**: `runner`, `app_name`, `title`, `art_cover`,
`art_square`, `art_background`, `art_logo`, `description`, `is_installed`,
`install.platform: "movie"`, and `extra.{stremioType, stremioTop,
watchCatalog, provider, watchId, imported}`. A downgrade reads the same file.
This build also writes `extra.{watchReleased, watchRuntime, watchPosition,
watchDuration, watchFinished, watchSource, watchStreams}`: a downgrade ignores
them, and the record keeps them for the screens that read a loaded library.
`watchSource` is the catalogue or provider that listed the title (restoring what
the wire key only implies), and `watchStreams` is the page's own probe (`true` a
stream, `false` none, absent never asked) — the only fact that says a title can
be played. `timestamps.lastPlayed` is normalized to `…T…Z` with milliseconds
(§9.9).

**The projection takes titles.** `refresh`, `append`, `replace` and `write_root`
take `&[Title]` and `titles(root)` returns them. One encode — `record_of(&Title)`
— is the only place the keys are written; the decode exists twice by necessity,
because the writer reads the raw files (`stored_item`) while the app reads a
loaded `Library` (`title_of_game`), and the two are held together by a test that
pits them against each other on one root. `MovieItem` remains as that wire
record — the Heroic shape — and nothing else builds one.

**This machine's own records are folded in before the write.** A refresh reads
`movies.json` once (`progress::LocalProgress`) and folds each record into its
title (`Titles::fold_local_progress`): a mark made on the page, or a playback
the provider has not been told about yet, is the state the screens show — the
later stamp wins, the way `Title::merge` decides two providers' states. Without
it a cover would say "Not watched" under a page that says "Watched", because
`movies.json` is the only place a hand mark lives.

**5.1a The keys are not the identity.** They are `app_name = {owner}:{id}`,
`local_meta = app_name`, `metadata = {owner}:{app_name}` (so
`stremio:stremio:tt1` — the double prefix is frozen on purpose), and
`timestamps = app_name`. The id carry the **kind nowhere**: a film and a series
sharing an id collide in every one of those maps. The library used to mask this
by keeping the first of the pair; `Titles` now merges them, so P4 must keep a
rule for the collision and a test, because two titles with one wire key would
otherwise overwrite each other.

**`is_installed` means "a stream provider here can play it"**, documented in
code at the write site. A placeholder is `false` with `extra.imported: true`.

### 5.1 The round trip is permanent

`Titles → projection → Titles` is not a one-way street: `append`, `replace` and
`refresh` read the file back through `titles()`, and `kept()`/`replace()` parse
`extra.watchCatalog` (the `collection:` prefix decides retention;
`replace` decides page membership). So the plan names the seam rather than
pretending it disappears:

- `title_of_game(&CollectionGame) -> Title` reconstructs a title from a
  projection record; `Titles::from_projection` rebuilds a library from a root.
  The adapter lives in `heronite-watch`, not beside `Title`: `CollectionGame` is
  upstream's type and `watch-core` may not depend on `heronite-data`.
- It is lossy in exactly four places: one record per title means `providers`
  comes back with one entry; one `watchCatalog` means `rails` comes back with one
  mark; the projection records no `addon`, so a feed mark comes back with an
  empty `source`; and the record stores the id without its **space** — a
  playable record re-derives it from its provider, so only a *placeholder* with a
  non-IMDb id comes back canonical. All four are stated, not accidental.
- **`extra.watchCatalog` must stay a single string that `kept()` and `replace()`
  can parse.** A `Vec<RailMark>` is reconstructed from it (`RailMark::as_wire`/
  `from_wire`); adding a second key would be a new frozen key and is out of
  scope.
- A provider that fails leaves the file untouched
  (`watch/src/library.rs:223-225`), so the round trip is also the recovery
  path: `Titles::from_projection` is what re-reads a library nobody just built.

### 5.2 One record per title, then per provider

Before P6: one record per title, chosen by registry order. **Now (P6a):** one
record per playable provider, linked in `local_library/copies.json` so the
Collection collapses them to one card (`group_copies` defaults to on,
`crates/data/src/ui_state.rs:207`) with its copies badge. The grid collapsing is
upstream's; the "Play with \<provider\>" action is still P6b, because with a
single provider there is no second copy to press.

The copies write path honours the prerequisites: Watch records carry none of the
ids `copies::groups` links by (`ids()` looks for `igdb:`/`steam:`), so grouping
is only ever by the links we write; the writer adds links but never overrides a
person's `apart`, `preferred` or other links; and the root the writer writes is
the one the screens read. The fold back is by `TitleKey`, so it joins records
whose id is the same *in the same space*: a title known only in a provider's own
id space re-derives that space from the record's provider and groups with itself,
while two providers that know one title under different spaces stay two titles
until a lookup joins them (§9.3).
(`active_data_paths()`), not a second copy.

## 6. Phases

Each phase is independently shippable, green on its own, and touches only the
column it names. P1–P3 are `crates/watch*`; P4–P5 are ours; P6 is gated on a
second provider.

**P1 — the shape.** `crates/watch-core/src/title.rs`: `TitleKind`, `ProviderId`,
`IdSpace`, `TitleId`, `TitleKey`, `Art`, `RailMark`, `Watched`, `Title`,
`Title::merge`, `Title::available`. Today the file has `provider: Option` +
`available: bool` and nothing outside its tests uses it. Acceptance: a film in
two providers is one title with two providers; a placeholder is one with none;
"show"/"tv" are series; first-non-empty merge; no serde derive. Tests: merge
per field, placeholder, kinds, id spaces.

**P2 — the library and the boundary (ours).** `crates/watch/src/library.rs`,
renamed from `collection.rs`: `Titles` keyed by `TitleKey`, insertion-ordered
(the rails' order is the grid's order — a map would sort by id), built through
`Titles::from_parts(Parts { rails, collections, shelf, placeholders })`; plus the
two adapters, `Title::of_item` in `watch-core` and `title_of_game` in
`heronite-watch` (where `CollectionGame` is visible), and the wire codec
`RailMark::{as_wire, from_wire}`. Acceptance: the titles equal the `MovieItem`
set the old path produced (ids, order, availability, first mark — pinned by a
golden test), and **the production `items()` builds that same `Titles`**, so the
test is not a parallel implementation; two rails carrying one title give one
title; the round trip is lossless except for the four documented losses. No
network in tests. **This comes before P3 on purpose**: the projection and the
catalogue pages still speak `MovieItem`, so providers cannot switch first
without a back-adapter — the thing this phase retires.

**P3 — providers emit titles (two slices).**

**P3a — the sources (done).** `watch-addon`'s `MetaPreview::into_title` and
`AddonClient::catalog` (which now answers previews), `watch-catalogs`' Cinemeta,
`watch-tmdb`'s `item`, and Stremio's `from_library`/`continue_watching` build
`Title`s; `Rail.items` and `CatalogPage.items` carry them. The catalogue source
stamps the provider that plays its titles (`PLAYS_WITH`), the feed mark comes
from the source's own id, and `ProviderId::declared` is the only trusted
constructor. The assembly merges two observations instead of dropping the second
(and `top` is `any`, so a Top placement survives). `items()` now builds the
library through `Titles::from_parts` — the duplicate `assemble`/`mark_collections`
path is gone — and the projection writes the titles themselves, marking one with
no provider as the writer's placeholder (`addon: "imported"`, which is what
`is_installed: false` and F-019's grey rule read).
Acceptance met: the sources' tests assert titles, the watch crate's tests assert
the rails/order/marks and the placeholder's projection, and
`cargo test --workspace` is green.

**P3b — the shelf (done).** `watch-core::personal` holds `ShelfTitle`, the frozen
snapshot of §3.1: its wire names are the old ones (`year`, art flattened), so an
old `watch-personal.json` loads and a downgrade reads it; `Title::of_shelf` adapts
one; `watch::personal`, `watch::import`, `watch-state::personal` and the app's
film page and toggles follow. `watch-imported.json`'s shape is untouched.
Acceptance met: the shelf's own test reads a file the previous build wrote and
checks the keys it writes back.

**P4 — the projection (done).** `crates/watch-state/src/collection.rs` takes
`&[Title]` everywhere — `refresh`/`append`/`replace`/`write_root` in,
`titles(root)` out — with one encode (`record_of`) and one raw decode
(`stored_item`), and `title_of_game` held to the latter by a parity test.
`item_of` is gone; `MovieItem` survives as the record. The frozen
keys, the single `watchCatalog` string and the double-prefixed metadata key are
unchanged, `lastPlayed` is normalized, and a placeholder round-trips as one
(written `is_installed: false` with `extra.imported: true`, read back with no
provider). The film/series collision is explicit and tested: the wire key carries
no kind, so the first title wins. The shelf's **reads** are canonical —
`watchlisted`/`favourited` through `same_title`, and `membership_any` for the
ledger — so a title kept from one provider reads as
kept when another provider's card asks; the ledger key stays `kind:provider:id`
(landed in `085ad29e`/`297d62a7`).

**P5 — the screens read titles (done).** `app/src/fork/watch/sources.rs` is the
one reader of the projection's keys: it gains `title(game)` (the typed view,
`title_of_game`) and builds the shelf snapshot with `Title::to_shelf`, replacing
the hand-written field mapping (`PersonalFields`/`personal_snapshot` are gone).
The cheap accessors (`id`, `kind`, `imported`, `rail_marker`, `year`) remain the
fast views of the same record, documented as such. The duplicate `imported()`
helper in upstream's `data.rs` is gone — the assignment calls
`sources::imported`, which is what shrank `F-019` from +19/−6 to +12/−6.

**`TitleRef` is deliberately left as it is.** The plan said P5 would make it a
`TitleKey`, and the code says no: a metadata source is selected *by provider*
(`MetaDescriptor::covers`, `crates/watch-core/src/metadata.rs:67`) and the details cache
is namespaced by it (`watch-state/src/details.rs:60`). Today every descriptor's
`applies_to` is empty, so `covers` accepts everything and the provider's only
live effect is the cache namespace — but the seam exists for the day a source
answers for one provider and not another, and collapsing it now would remove that
before anything needs it. Changing it would touch six crates for a cache-layout
preference. Recorded here as a decision, not an omission.

**P6 — the multi-source projection (two slices).**

**P6a — the projection (done).** `write_root` writes one record per playable
provider and links them in `local_library/copies.json`, read-modify-write: a pair
the person ungrouped stays ungrouped, their `preferred` list and other links are
kept, and a title that gains a provider replaces its smaller link instead of
adding a second. `stored`/`titles` folds the records back into one title, so the
projection round-trips a two-provider title as one title with both sources. The
watch state is written **once per title**, on one copy: upstream's copies total
sums every copy of a group and renders "N in all" on the grouped card, so
repeating the minutes would double it. Acceptance met for the file layer: two
providers give two records, one link, one title back, one timestamp; an ungrouped
pair is not relinked.

**P6b — the player (gated on a real second provider).** The `copies_actions` →
`PlayCopy` branch (`F-020`), so "Play with \<provider\>" opens that provider's
streams instead of the store client's launch. It is reserved, not built: with one
provider there is no copy menu to exercise, and an untested upstream edit is a
merge cost without a benefit. When a second provider ships, its acceptance is:
two providers give one card with the copies badge; "Play with" opens that
provider's streams; grouping off gives one card per provider; a refresh keeps the
person's grouping; the film page and the watchlist rail agree across providers.

**P7 — close-out (done).** The gates ran clean: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace` (i18n coverage included, and run on its own), and
`cargo check -p heronite`. The register review over this work's diff found four
upstream-owned files and **all four are registered**: `load.rs`/`mod.rs` (the
module rename, F-006), `console/sources.rs` (the watchlist rail, F-008/F-016)
and `data.rs` (the flag's assignment, F-019, now +12/−6). Everything else it
touches is `crates/watch*`, `app/src/fork/watch/**` or the two fork-only app
modules, which the register does not list because they are additions.
`F-019`'s file list and line count were re-measured over the seven files with
`git diff eecd5c2a --numstat` **on the working tree** — where the helper removal
shows, +12/−6. Committed at `fork/main` it is still +19/−6 until this lands.

## 7. What we deliberately do not do

- **Rewrite the renderer.** `card`/`tile`/`carousel`/`spotlight` stay upstream's;
  they keep eating `CollectionGame`-shaped data produced by our projection. The
  console's detail page is the one exception, and it is fork-owned
  (`docs/WATCH-PAGE.md`).
- **Rename upstream's crates or types.** `CollectionGame` remains the *view*
  type; `heronite_data::Library` keeps its name — which is why our collection is
  `Titles` in `watch::library`, never a second `Library`. The module rename does
  move the app-facing `heronite_watch::collection` path, so P2 updates
  `docs/WATCH-CRATES.md`'s table with it.
- **Keep a second identity type.** The store's `TitleKey` is the identity; the
  wire keys are derived from it (and carry no kind), `same_title` is `(kind, id)`
  over the shelf's snapshots, and no `(provider, kind, id)` tuple is used as one.
  `TitleRef` stays a *provider-scoped descriptor* for metadata sources (§P5), not
  a second identity.
- **Change what `installed` means for games.** The repurpose is confined to the
  watch library writer.
- **Move the film page onto upstream.** It stays fork-owned, as one subsystem
  (`docs/WATCH-PAGE.md`): hosting it inside upstream's five detail-page layouts
  was measured larger and lands in the files upstream rewrites every sync.
- **Send anything upstream.** No issue and no pull request for the fork's own
  features: the seams of §2 are fork-side patches we register and merge
  ourselves. The single exception is the generic console extension point
  `docs/WATCH-PAGE.md` offers under the upstream-first rule, which the fork
  never depends on.

## 8. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Shared-file edits accumulate | the budget in §2, measured in seam sites per file and machine-checked in `app/src/fork/seams.rs`; each registered |
| The projection's read-back path is forgotten | §5.1 names it; P2's acceptance tests the round trip |
| A rail mark is lost through the single `watchCatalog` | §5.1; the persisted mark is the one `kept`/`replace` parse; P4 asserts it |
| A film and a series share an id on the wire | §5: the keys carry no kind; P4 keeps an explicit first-wins rule and a test |
| Two models drift while P1–P3 are in flight | P2's parity test compares them; after P3 there is one |
| A second provider splits a title's history | reads go canonical in P4 (`membership_any`); progress stays `(provider, id)` by design |
| A second provider shows a title twice | P6 links the copies; `group_copies` is on by default |
| Themes stop seeing what they expect | the projection keeps every key and `install.platform: "movie"` |
| A placeholder goes un-dimmed under a theme | `F-019` keeps the field and the five conditions |
| Upstream merges rename a field we adapt to | the adapters are `sources.rs` and the writer; no domain change |

## 9. Decisions taken

1. **`available`**: the word stays — it is the product's own — as a derived
   method over `providers`; no stored flag.
2. **Identity**: `TitleKey` includes the **id space**, so a YouTube-only title
   is representable today rather than at a future tripwire; matching across
   spaces is a lookup, never a merge.
3. **No `known` set.** Provenance is `rails` for a listing and the personal
   ledger / `watch-imported.json` for an import. Dropping it removes the
   unenforceable `playable ⊆ known` invariant and the mixed actor kinds
   (stream provider vs catalogue vs tracker) in one type.
4. **`rails`**: `Vec<RailMark>`, first mark names the title; the writer persists
   the first, as `mark_collections` already does. The kind of rail is in the
   type, not in a string prefix — though `kept()` still *parses*
   `collection:<id>` out of the wire, so the persisted string is the one the
   reader expects.
5. **`top` stays a field** even though it is wire-only: the single persisted
   mark cannot carry both "which rail" and "was it the Top feed",
   and deriving it would lose one of them.
6. **`CardVm.imported`** stays (`F-019`); the duplicated helper goes in P5.
7. **Multi-source**: modelled in the domain now, projected in P6, because the
   projection is what costs upstream lines and shelf identity.
8. **No serde on `Title`**: `ShelfTitle` and `Placeholder` are the only
   persisted item shapes.
9. **`Watched.at` is a `DateTime<Utc>`**, not the provider's string: "later
   wins" needs an instant, and the provider already holds one
   (`watch-stremio/src/datastore.rs:51`). The writer prints it back as
   `…T…Z` with milliseconds, because upstream orders the wire strings
   lexicographically (`crates/data/src/copies.rs:374`,
   `carry_over.rs:214`) and a `+00:00` offset would sort beside `Z` rather than
   after it.

## 10. Gates

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` (which includes the i18n coverage test), and
`cargo check -p heronite` after each phase. A phase that changes the shared
column must also say why in `docs/FORK.md`, and the close-out reviews
`git diff $(git merge-base upstream/main fork/main)..fork/main --name-only` so
the deviation surface is what this document claims.

## 11. What the reviews changed

Two read-only reviews (architecture under the Rust guidelines, and a
claim-by-claim check against the tree) rejected the first revision. Corrected:
the deviation footprint (7 files and **five** grey sites, not three — the two
`console/layouts` files were missing); "P5 shrinks by 8 lines" (it is 7); the
wire-key sentence (§5 said identity was `(provider, kind, id)` — the keys carry
no kind at all); the list-slice commits (§5 cited three that were not the
ledger); "nothing parses `collection:` any more" (it does); `copies` as a
`CollectionGame` field (it is `Library.copies`); and `ShelfTitle` presented as
if it existed (it is new). Added: the id space, the round trip (§5.1), merge
semantics per field, the film/series key collision, P2 before P3, canonical
ledger reads, and the P6 copies prerequisites.

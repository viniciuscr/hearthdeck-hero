# Watch titles — the transition plan

Status: **plan for review** (2026-11-19, second revision). Phase 1 is partly
implemented; nothing else is. Two independent reviews of the first revision
found factual errors (the deviation footprint, the wire keys, the phase order)
and three model holes (`TitleId` was IMDb-only, `Sources` carried an
unenforceable invariant, and the projection's read-back path was missing). This
revision fixes them; §11 records what the reviews changed.

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
explicit, registered budget (§2) and asks upstream for nothing.

## 2. Ours and upstream's

| Ours (fork-owned) | Upstream's (shared) |
|---|---|
| `crates/watch-core`, `crates/watch-state`, `crates/watch`, `crates/watch-*` | `app/src/collection/{vm,data,card,tile,carousel,list_row}.rs` |
| `app/src/fork/watch/**` | `app/src/collection/console/{game,layouts}/**` (the renderer) |
| `app/src/collection/console/watch_page.rs` (the film page) | `crates/data/**` (`CollectionGame`, `Library`, `copies`, `json_store`) |
| `crates/watch-state/src/collection.rs` (the **watch library writer**) | the theme system, widgets, navigation, image loader |

**The rule this document exists to fix:** *shared files never learn a Watch
noun.* A Watch concept is adapted at the boundary into the shapes upstream
already consumes.

### The upstream budget

Every line this transition touches in upstream's files, and nothing else:

| Seam | Files | Footprint | Why it cannot be ours |
|---|---|---|---|
| the placeholder flag (`F-019`) | `app/src/collection/{vm,data,card,list_row}.rs`, `console/tile.rs`, `console/layouts/carousel.rs`, `console/layouts/spotlight/row.rs` | one `CardVm` field (+4 lines with its test fixture), one assignment, a 3-line helper `data.rs` duplicates from `sources.rs` (+8 lines), **five** one-line grey conditions | the card model and the grey rule are upstream's renderer; "a placeholder is always dimmed, whatever the theme says" has no other expression |
| a provider's display name (`F-007`) | `crates/data/src/library.rs` (`source_label`) | **one line per stream provider** ("Stremio" is there) — an English literal the i18n coverage gate cannot see | `source_label` is upstream's table; an unknown source falls back to "Local" |
| "Play with \<provider\>" (`F-020`) | `app/src/collection/copies.rs` → `GameAction::PlayCopy` | one branch, when a second provider ships | `PlayCopy` ends in the store client's launch (`app/src/collection/game_actions.rs:424`), which cannot start a Watch stream |

P5 **reduces** the first: the duplicated helper goes and `data.rs` keeps only
its assignment (8 added lines → 1). The second cannot shrink below one line per
provider. The third is deferred — confirmed necessary, because `PlayCopy` never
reaches Watch today.

No upstream change is requested for any of them: each is a fork-side patch with
a `Fork-Seam` trailer and a `docs/FORK.md` row. Today's footprint is 7 files:
+19/−6 lines, one field and five conditions.

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

pub struct Watched { pub at: Option<String>, pub minutes: f64 }

pub struct Title {
    pub key: TitleKey,
    pub name: String,
    pub release: Option<String>,   // as the library prints it ("1999", "2011–2019")
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
  already assumes this (`watch_playback.rs:638` keeps `imdb` only for `tt…`).
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
  - name, release, description, genres, art: the **first non-empty** value
    wins, so a provider that answers with a poster fills a title another left
    bare; `merge` is therefore idempotent but **not** commutative.
  - `providers` and `rails`: union, order preserved (`rails` keeps the first
    mark as the one that names the title).
  - `watched`: the later `at` wins, with its `minutes`; a tie keeps the left.
  - `top`: `any`.
- **Episodes are not a kind.** Catalogue pages carry `movie|series`; an episode
  is a `metadata::Episode` inside a series, with per-episode state in
  `movies.json`. `TitleKind` stays two, because `type:movie`/`type:series` tab
  sources, `stremioType` and the writer's keys all mean the two.
- **`runtime` is dropped.** Nothing reads `MovieItem::runtime`; the film page
  takes it from `TitleDetails`.
- **A catalogue source is not a stream provider.** A catalogue page lists
  titles; the providers that can resolve a stream for a title's id play it —
  today Stremio alone, which is why a Cinemeta or TMDB item is playable by
  Stremio and a YouTube track would not be. `RailMark::Feed::source` names the
  catalogue; `providers` names who plays.
- **`ProviderId` comes from the registry**, not from a raw string: the only
  constructors are the providers `registry::providers()` returns, so
  `"imported"` stops being a pseudo-provider and a typo cannot become an owner.

### 3.1 The one persisted shape

`watch-personal.json` is user data a downgrade must still read, so the shelf
keeps a frozen snapshot — a **new** type, `ShelfTitle` in
`watch-core::personal`: kind, id, name, release, art, description, genres, and
the provider as *provenance*. It is a remembered card, not a library record:
carrying `providers` or `rails` would only store what the next refresh is about
to correct. Its serde accepts the old `MovieItem` names, so a file written
before the transition still loads. `watch-imported.json`'s `Placeholder` is its
own shape and does not change at all.

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

**The keys are not the identity.** They are `app_name = {owner}:{id}`,
`local_meta = app_name`, `metadata = {owner}:{app_name}` (so
`stremio:stremio:tt1` — the double prefix is frozen on purpose), and
`timestamps = app_name`. The id carry the **kind nowhere**: a film and a series
sharing an id collide in every one of those maps. `assemble` masks this today by
keeping the first of the pair (`watch/src/collection.rs:182-193`); P4 must keep
a rule for it and a test, because a `Titles` keyed by `(kind, id)` would
otherwise hand the writer two titles and let the second overwrite the first.

**`is_installed` means "a stream provider here can play it"**, documented in
code at the write site. A placeholder is `false` with `extra.imported: true`.

### 5.1 The round trip is permanent

`Titles → projection → Titles` is not a one-way street: `append`, `replace` and
`refresh` read the file back through `stored()`, and `kept()`/`replace()` parse
`extra.watchCatalog` (the `collection:` prefix decides retention;
`replace` decides page membership). So the plan names the seam rather than
pretending it disappears:

- `Title::of_game(&CollectionGame) -> Title` reconstructs a title from a
  projection record; `Titles::from_projection` rebuilds a library from a root.
- It is lossy in exactly two places: one record per title means `providers`
  comes back with one entry, and one `watchCatalog` means `rails` comes back
  with one mark. Both are stated, not accidental.
- **`extra.watchCatalog` must stay a single string that `kept()` and `replace()`
  can parse.** A `Vec<RailMark>` is reconstructed from it plus `extra`'s
  `addon`; adding a second key would be a new frozen key and is out of scope.
- A provider that fails leaves the file untouched
  (`watch/src/collection.rs:33-36`), so the round trip is also the recovery
  path: `Titles::from_projection` is what re-reads a library nobody just built.

### 5.2 One record per title, then per provider

Today: one record per title, chosen by registry order. P6, once a second
provider ships: one record per playable provider, linked in
`local_library/copies.json` so the Collection collapses them to one card
(`group_copies` defaults to on, `crates/data/src/ui_state.rs:207`) with its
copies badge and "Play with \<provider\>".

The copies write path has prerequisites, all in P6's acceptance: Watch records
carry none of the ids `copies::groups` links by (`ids()` looks for `igdb:`/
`steam:`), so grouping is only ever by the links we write; the writer must own
`links` while never overriding a person's `apart`, `preferred` or ungroup;
`totalPlayed` is written on one copy only, or the totals double-count; and the
root the writer writes must be the one the screens read
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
(the rails' order is the grid's order — `BTreeMap` would sort by id), built
through a pure `Titles::from_parts(rails, shelf, placeholders)`; plus
`Title::of_item` and `Title::of_game` (§5.1), which are the two adapters the
rest of the plan leans on. Acceptance: the titles equal the `MovieItem` set the
old path produces (ids, order, availability); two rails carrying one title give
one title; the round trip is lossless except for the two documented losses. No
network in tests. **This comes before P3 on purpose**: the projection and the
catalogue pages still speak `MovieItem`, so providers cannot switch first
without a back-adapter — the thing this phase retires.

**P3 — providers emit titles.** `watch-stremio`, `watch-tmdb`,
`watch-catalogs`, `watch-addon` and `watch-core::personal` speak `Title`: no
`String` kind, no empty-provider-means-Stremio default, no `owner()`, one
`IMPORTED` const. The shelf moves to `ShelfTitle`, old names aliased.
Acceptance: provider fixtures parse to the same titles; the boundary adapters
from P2 are deleted; an old shelf file still loads; `watch-imported.json`'s
shape is untouched.

**P4 — the projection (ours).** `crates/watch-state/src/collection.rs` takes
`Titles` and writes the frozen keys, keeping the film/series collision rule
(§5) and the single `watchCatalog` string. The personal ledger keeps its
`kind:provider:id` key (landed in `085ad29e`/`297d62a7`), but its **reads**
become canonical through `membership_any`: a title watchlisted from one provider
must read as watchlisted from another, and a sync entry stored under the empty
provider must do the same. Acceptance: the app renders identically for a normal
title and for a placeholder; the keys are asserted against a golden record; a
downgrade build reads the file; a film and a series sharing an id follow the
stated rule; the shelf answers across providers.

**P5 — the screens read titles (ours).** `app/src/fork/watch/sources.rs` becomes
the single reader, `Title::of_game`; the film page, filters, panel and playback
consult it instead of `extra` keys; `TitleRef` (`watch-core::metadata`) takes a
`TitleKey` instead of `(provider, id, kind)`, so details are cached per title.
The duplicate `imported()` helper in upstream's `data.rs` goes; the VM field and
the five conditions stay (`F-019`). Acceptance: the film page, marks, trailer
hint, personal flags and the watchlist rail are identical; `sources.rs` plus the
writer are the only files that read `extra`; the shared-file diff shrinks to one
line in `data.rs`.

**P6 — the multi-source projection** (only once a second stream provider ships).
One record per playable provider, `copies.json` links merged with the person's
own `links`/`apart`/`preferred`, and the `copies_actions` → `PlayCopy` branch
(`F-020`). Acceptance: two providers give one card with the copies badge; "Play
with" opens that provider's streams; grouping off gives one card per provider; a
refresh keeps the person's grouping; playtime is not double-counted; the film
page and the watchlist rail agree across providers.

**P7 — close-out.** The gates (§10), the register review, this document, and the
companions that still describe `MovieItem` — each updated by the phase that
changes what it describes, not in one sweep.

## 7. What we deliberately do not do

- **Rewrite the renderer.** `card`/`tile`/`carousel`/`spotlight` stay upstream's;
  they keep eating `CollectionGame`-shaped data produced by our projection.
- **Rename upstream's crates or types.** `CollectionGame` remains the *view*
  type; `heronite_data::Library` keeps its name — which is why our collection is
  `Titles` in `watch::library`, never a second `Library`. The module rename does
  move the app-facing `heronite_watch::collection` path, so P2 updates
  `docs/WATCH-CRATES.md`'s table with it.
- **Keep a second identity type.** `TitleRef`, `same_title` and the wire keys are
  all derived from `TitleKey`; no `(provider, kind, id)` tuple survives in the
  domain.
- **Change what `installed` means for games.** The repurpose is confined to the
  watch library writer.
- **Move the film page onto upstream.** It is already ours and stays there.
- **Send anything upstream.** No issue, no pull request: the seams of §2 are
  fork-side patches we register and merge ourselves.

## 8. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Shared-file edits accumulate | the budget in §2 (7 files, +19/−6); each registered; P5 shrinks one |
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

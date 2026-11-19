# Watch titles — the transition plan

Status: **plan for review** (2026-11-19, revised). Phase 1 is partly
implemented; nothing else is. This revision follows a review against the tree:
§2's register claim was untrue, §3's "adapter" was never wired, `available`
duplicated `provider`, and `Title` as first written could carry neither the
writer (description, genres, placements, watch state) nor the screens.

Companion to `docs/WATCH.md` (the space), `docs/WATCH-IMPORT.md` (sync) and
`docs/WATCH-CRATES.md` (the crate boundary). `docs/FORK.md` owns the deviation
register this plan feeds.

## 1. Why

Watch began inside a collection whose unit is **a game from a store**:
`CollectionGame` with `runner`, `app_name`, `source_label`, `installed`,
`copies`, `family`. Watch's data had to travel in `extra` (`watchId`,
`stremioType`, `watchCatalog`) and `installed` had to be repurposed to mean "a
stream source can play this".

That is the smell this transition removes. The structure stays — a library,
rails, tiles, progress, focus, themes — only the nouns and the model become
Watch's: **stream providers** and **movies/series**.

The transition's binding constraint is upstream. Every line we add in a shared
file is a conflict to reconcile at every upstream merge, so the plan keeps that
surface to an explicit, registered budget (§2) and asks upstream for nothing.

## 2. Ours and upstream's

| Ours (fork-owned) | Upstream's (shared) |
|---|---|
| `crates/watch-core`, `crates/watch-state`, `crates/watch`, `crates/watch-*` | `app/src/collection/{vm,data,card,tile,carousel,list_row}.rs` |
| `app/src/fork/watch/**` | `app/src/collection/console/{game,layouts}/**` (the renderer) |
| `app/src/collection/console/watch_page.rs` (the film page) | `crates/data/**` (`CollectionGame`, `Library`, `json_store`) |
| `crates/watch-state/src/collection.rs` (the **watch library writer**) | the theme system, widgets, navigation, image loader |

**The rule this document exists to fix:** *shared files never learn a Watch
noun.* A Watch concept is adapted at the boundary into the shapes upstream
already consumes.

### The upstream budget

Every line this transition touches in upstream's files, and nothing else:

| Seam | File | Footprint | Why it cannot be ours |
|---|---|---|---|
| the placeholder flag (`F-019`) | `app/src/collection/{vm,data,card,list_row}.rs`, `app/src/collection/console/tile.rs` | one `CardVm` field, one assignment (plus a helper `data.rs` duplicates from `sources.rs`), three one-line grey conditions | the card model and the grey rule are upstream's renderer; "a placeholder is always dimmed, whatever the theme says" has no other expression |
| a provider's display name (`F-007`) | `crates/data/src/library.rs` (`source_label`) | **one line per stream provider** ("Stremio" is already there) | `source_label` is upstream's table; an unknown source falls back to "Local" |
| "Play with \<provider\>" (`F-020`, only once a second provider ships) | `app/src/collection/copies.rs` (`copies_actions` → `GameAction::PlayCopy`) | one branch | the copies menu and its action are upstream's |

P5 **reduces** the first (the duplicated helper goes, −8 lines); the second
cannot shrink below one line per provider; the third is deferred until a second
provider exists. Today's footprint is one field, three conditions and the
`stremio` label line — `F-019` was missing from the register until this
revision.

No upstream change is requested for any of them: each is a fork-side patch with
a `Fork-Seam` trailer and a `docs/FORK.md` row.

## 3. The model

`crates/watch-core/src/title.rs`, as it will be after P1:

```rust
pub enum TitleKind { Movie, Series }        // "show"/"tv" map to Series
pub struct TitleId(String);                 // the canonical id ("tt0133093")
pub struct ProviderId(String);              // a stream provider: Stremio today
pub struct Art { poster, background, logo }

/// Where a title was listed. The first mark names it.
pub enum RailMark {
    Feed { source: String, catalog: String }, // a catalogue page ("tmdb", "movie:top:Popular")
    Collection(String),                       // a rail the person turned on
}

/// Who can play a title, and who merely knows about it. `playable ⊆ known`.
#[derive(Default)]
pub struct Sources {
    pub playable: BTreeSet<ProviderId>,
    pub known: BTreeSet<ProviderId>,
}

pub struct Watched { pub at: Option<String>, pub minutes: f64 }

/// A title's identity: what makes two observations the same title.
pub struct TitleKey { pub kind: TitleKind, pub id: TitleId }

pub struct Title {
    pub key: TitleKey,
    pub name: String,
    pub release: Option<String>,          // as the library prints it ("1999", "2011–2019")
    pub art: Art,
    pub description: Option<String>,
    pub genres: Vec<String>,
    pub rails: Vec<RailMark>,
    /// A placement, not identity: Cinemeta's Top membership.
    pub top: bool,
    pub sources: Sources,
    pub watched: Option<Watched>,
}
```

Six consequences worth stating:

- **Identity is `(kind, id)`.** The provider that plays a title is not part of
  what the title *is*; the same film may play from two providers, and it is one
  title with two sources.
- **`available` is derived**, never stored: `Title::available()` is "a playable
  source exists". The word is the product's own — the shipped copy already says
  "Not currently present in any available stream source" — but a field would let
  the two disagree.
- **A placeholder is a `Title` with no playable source.** `sources.known` still
  records who told us, which the current projection throws away (the imported
  rail builds `provider: ""`). No flag is repurposed.
- **`Title::merge` is the only way two observations become one**: union
  `sources` and `rails`, first wins name, art, description, genres.
- **Episodes are not a kind.** Catalogues carry `movie|series`; an episode is a
  `metadata::Episode` inside a series, with per-episode state in `movies.json`.
  `TitleKind` stays two, because `type:movie`/`type:series` tab sources,
  `stremioType` and the writer's keys all mean the two.
- **`runtime` is dropped.** Nothing reads `MovieItem::runtime`; the film page
  takes it from `TitleDetails`.

The shelf is the one exception, and deliberately: `watch-personal.json` is user
data that a downgrade must still read, so it keeps a frozen snapshot —
`ShelfTitle` in `watch-core::personal`: what a card needs (kind, id, name,
release, art, description, genres) plus the provider as *provenance*. It is a
remembered card, not a library record: carrying `sources` or `rails` would only
store what the next refresh is about to correct. Its serde accepts the old
`MovieItem` names, so a file written before the transition still loads.

## 4. Vocabulary map

| Today (games' words) | Watch's word | Where |
|---|---|---|
| `runner` | `Sources::playable` (`ProviderId`) | domain |
| `app_name` (`owner:id`) | the projection's key, **derived**; never stored on a title | writer |
| `installed` | "a playable source exists" (`Title::available()`); `is_installed` in the projection | domain |
| `source_label` ("Steam", "GOG") | the provider's name ("Stremio") | screens (upstream's table, `F-007`) |
| `extra.watchId` | `TitleKey::id` | domain |
| `extra.stremioType` | `TitleKey::kind` | domain |
| `extra.watchCatalog` | `Title::rails` | domain |
| `extra.stremioTop` | `Title::top` | domain |
| `extra.imported` | `Sources::playable` is empty | domain |
| `MovieItem.addon` ("cinemeta") | `RailMark::Feed::source` | domain |
| store / game | stream provider / movie, series | everywhere |

Two rows need care. `app_name` is not a title field: it is `<owner>:<id>`, the
key `CollectionGame` and the projection use, and it stays derived exactly as
today. `installed` is not renamed for games — the repurpose is confined to the
watch library writer, which is ours.

## 5. What upstream sees (the projection)

`crates/watch-state/src/collection.rs` projects titles into the Heroic shapes
`heronite-data` reads, under `<cache>/watch-library`, regenerated on every load
and never merged with the games' library.

- **The keys are frozen**: `runner`, `app_name`, `title`, `art_cover`,
  `art_square`, `art_background`, `art_logo`, `description`, `is_installed`,
  `install.platform: "movie"`, and `extra.{stremioType, stremioTop,
  watchCatalog, provider, watchId, imported}`. A downgrade reads the same file.
- **`is_installed` means "a stream source here can play it"**, documented in
  code at the write site. A placeholder is `false` with `extra.imported: true`.
- **Identity in the projection is `(provider, kind, id)`.** `refresh`,
  `append`, `replace` and `kept` must agree: today `assemble` dedupes on
  `(kind, id)` and silently drops a second copy of the same title.
- **One record per title today, one per playable provider in P6.** P6 links the
  copies in `local_library/copies.json` — upstream's own "one game, several
  stores" — so the Collection collapses them to one card (`group_copies`
  defaults to on) with the copies badge and "Play with \<provider\>".

## 6. Phases

Each phase is independently shippable, green on its own, and touches only the
column it names. P1 and P2 are `crates/watch*`; P3–P4 are ours; P5 is the
adapter; P6 is gated on a second provider.

**P1 — the shape.** `crates/watch-core/src/title.rs`: `TitleKind`, `TitleId`,
`ProviderId`, `Art`, `Sources`, `RailMark`, `Watched`, `Title`, `Title::merge`,
`Title::available`. Today the file has `provider: Option<ProviderId>` +
`available: bool` and nothing outside its tests uses it; P1 finishes the shape.
Acceptance: a film in two providers is one title with two playable sources; a
placeholder is one with none but with `known` set; "show"/"tv" are series;
`available()` derives. Tests: merge, placeholder, kinds, serde of an old item.

**P2 — providers emit titles.** `watch-stremio`, `watch-tmdb`,
`watch-catalogs`, `watch-addon`, `watch-core::personal` and the placeholders
speak `Title`: no `String` kind, no empty-provider-means-Stremio default, no
`owner()`, one `IMPORTED` const for the marker. The shelf keeps `ShelfTitle`
(§3), with the old names aliased. Acceptance: provider fixtures parse to the
same titles; the shelf file still reads an old entry.

**P3 — the library (ours).** `crates/watch/src/library.rs`, renamed from
`collection.rs`: `Titles`, keyed by `(kind, id)`, built from the provider rails,
the catalogue pages, the personal shelf, the collections and the placeholders
through a pure `Titles::from_parts(rails, shelf, placeholders)`. Acceptance: the
titles equal the `MovieItem` set the old path produces (ids, order,
availability); a title in two rails is one title; one record per title. No
network in tests.

**P4 — the projection (ours).** `crates/watch-state/src/collection.rs` takes
`Titles` and writes the frozen keys with `(provider, kind, id)` identity for a
record. The list-membership slice has landed (`ee21d8bc`, `2782891f`,
`3f0dfd57`) and keys the shelf ledger by `kind:provider:id`, writing sync
entries under an empty provider; P4 keeps that ledger key as it is while the
*library* identity becomes `(kind, id)`. Acceptance: the app renders identically
for a normal title and for a placeholder; a downgrade build reads the file; the
keys are asserted.

**P5 — the screens read titles (ours).** `app/src/fork/watch/sources.rs` becomes
the single reader, `Title::of_game(&CollectionGame)`, and the film page,
filters, panel and playback consult it instead of `extra` keys. The duplicate
`imported()` helper added to upstream's `data.rs` goes — the VM field and the
three conditions stay (`F-019`). Acceptance: the film page, marks, trailer hint,
personal flags and the watchlist rail are identical; `sources.rs` is the only
file that reads `extra`; the shared-file diff shrinks by 8 lines.

**P6 — the multi-source projection** (only once a second stream provider ships).
One record per playable provider, `copies.json` links merged with the person's
own `links`/`apart`/`preferred`, and the `copies_actions` → `PlayCopy` branch
(`F-020`) so "Play with \<provider\>" reaches the Watch picker. Acceptance: two
providers give one card with the copies badge; "Play with" opens that provider's
streams; grouping off gives one card per provider; a refresh keeps the person's
grouping; playtime is not double-counted in the totals.

**P7 — close-out.** The gates (§10), the register review, and this document.
The companions that still describe `MovieItem` (`docs/WATCH.md`,
`docs/WATCH-IMPORT.md`, `docs/WATCH-COLLECTIONS.md`) are updated by the phase
that changes what they describe, not in one sweep.

## 7. What we deliberately do not do

- **Rewrite the renderer.** `card`/`tile`/`carousel`/`spotlight` stay upstream's.
  They keep eating `CollectionGame`-shaped data produced by our projection.
- **Rename upstream's crates or types.** `CollectionGame` remains the *view*
  type; `heronite_data::Library` keeps its name, which is why our collection of
  titles is `Titles` in `watch::library`, not a second `Library`.
- **Change what `installed` means for games.** The repurpose is confined to the
  watch library writer.
- **Move the film page onto upstream.** It is already ours and stays there.
- **Send anything upstream.** No issue, no pull request: the seams of §2 are
  fork-side patches that we register and merge ourselves.

## 8. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Shared-file edits accumulate | the budget in §2; each registered; P5 shrinks one |
| Two models drift (a `MovieItem` and a `Title` disagreeing) | only while P1–P2 are in flight: P3's parity test compares them, and after P2 there is one |
| `(provider, kind, id)` identity creeps back into the domain | `Title` has no provider in its identity; the personal ledger is the only place a provider appears, as provenance; `TitleKey` is the tripwire |
| Themes stop seeing what they expect | the projection keeps every key and `install.platform: "movie"` |
| A second provider shows a title twice | P6 links the copies; `group_copies` is on by default; the person can ungroup |
| Upstream merges rename a field we adapt to | the adapter is `sources.rs` and the writer; no domain change |
| A placeholder goes un-dimmed under a theme | `F-019` keeps the field and the three conditions; P5 must not remove them |

## 9. Decisions taken

1. **`available`**: the word stays — it is the product's own — as a derived
   method over `Sources`; there is no stored flag.
2. **`rails`**: `Vec<RailMark>`, first mark names the title; the writer persists
   the first, as `mark_collections` already does. The kind of rail is in the
   type, not in a string prefix — the wire still carries `collection:<id>` in
   `extra.watchCatalog`, but nothing parses it any more.
3. **Provider-local ids** (`simkl`, `tmdb`): not on `Title`. A lookup that needs
   them goes through `(provider, id)` at the edge, which is how progress records
   already key. Tripwire: the day a provider serves titles under its own id,
   `TitleId` stops being IMDb-only and this decision is revisited.
4. **`CardVm.imported`**: kept, registered as `F-019`; the duplicated helper in
   `data.rs` is removed in P5.
5. **Multi-source**: modelled in the domain now (`Sources`), projected in P6,
   because the projection is what costs upstream lines and shelf identity.

## 10. Gates

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
warnings`, `cargo test --workspace` (which includes the i18n coverage test), and
`cargo check -p heronite` after each phase. A phase that changes the shared
column must also say why in `docs/FORK.md`, and the close-out reviews
`git diff <last-upstream-merge>..fork/main --name-only` so the deviation surface
is what this document claims.

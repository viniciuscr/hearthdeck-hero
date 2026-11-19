# Watch titles — the transition plan

Status: **plan for review** (2026-11-19). Phase 1 is implemented; nothing else is.
Companion to `docs/WATCH.md` (the space) and `docs/WATCH-IMPORT.md` (sync).

## 1. Why

Watch began inside a collection whose unit is **a game from a store**: `CollectionGame`
with `runner`, `app_name`, `source_label`, `installed`, `copies`, `family`. Watch's data
had to travel in `extra` (`watchId`, `stremioType`, `watchCatalog`) and `installed`
had to be repurposed to mean "a stream source can play this".

That is the smell this transition removes. The structure stays — a library, rails,
tiles, progress, focus, themes — only the nouns and the model become Watch's:
**stream providers** and **movies/series**.

## 2. Who owns what

The fork's seam is already clean. Every line we add on the right is a future merge
conflict with `upstream` (`RafaelGoulartB/Heronite`).

| Ours (fork-owned) | Upstream's (shared) |
|---|---|
| `crates/watch-core`, `crates/watch-state`, `crates/watch`, `crates/watch-*` | `app/src/collection/{vm,data,card,tile,carousel,list_row}.rs` |
| `app/src/fork/watch/**` | `app/src/collection/console/{game,layouts}/**` (the renderer) |
| `app/src/collection/console/watch_page.rs` (the film page) | `crates/data/**` (`CollectionGame`, `Library`, `json_store`) |
| `crates/watch-state/src/collection.rs` (the **watch library writer**) | the theme system, widgets, navigation, image loader |

**The rule this document exists to fix:** *shared files never learn a Watch noun.*
A Watch concept is adapted at the boundary into the shapes upstream already
consumes. The fork's only current exception is one `CardVm.imported` field and the
five one-line grey conditions that read it (`docs/FORK.md` records it so it can be
reverted).

## 3. The model (phase 1, implemented)

`crates/watch-core/src/title.rs`:

```rust
pub enum TitleKind { Movie, Series }        // "show"/"tv" map to Series
pub struct TitleId(pub String);             // the IMDb id ("tt0133093")
pub struct ProviderId(pub String);          // a stream provider: Stremio today
pub struct Art { poster, background, logo }

pub struct Title {
    pub id: TitleId,
    pub kind: TitleKind,
    pub name: String,
    pub year: Option<String>,
    pub art: Art,
    pub provider: Option<ProviderId>,   // None: nothing here plays it
    pub available: bool,                // == provider.is_some()
    pub rails: Vec<String>,
}
```

Two consequences worth stating:

- **A placeholder is a `Title` with no provider and `available: false`.** No flag is
  repurposed; the state is unrepresentable otherwise.
- **`Title::of_item(&MovieItem)`** is the adapter that keeps the current pipeline
  working. It is the only place that knows "an item with `addon == imported` is a
  placeholder" and "an item without a provider is Stremio's".

## 4. Vocabulary map

| Today (games' words) | Watch's word | Where |
|---|---|---|
| `runner` | `Title::provider` (`ProviderId`) | domain |
| `app_name` (`provider:id`) | `Title::id` + `provider` | domain |
| `installed` | `Title::available` | domain; `installed` survives only as the adapter's mapping for the shared renderer |
| `source_label` ("Steam", "GOG") | the provider's name ("Stremio") | screens |
| `extra.watchId` | `Title::id` | domain |
| `extra.stremioType` | `Title::kind` | domain |
| `extra.watchCatalog` | `Title::rails` | domain |
| `extra.imported` | `provider: None` | domain |
| store / game | stream provider / movie, series | everywhere |

## 5. Phases

Each phase is independently shippable, green on its own, and touches only the
column it names.

**P1 — the type (done).** `Title`, `TitleKind`, `TitleId`, `ProviderId`, `Art`,
`Title::of_item`. Acceptance: a library item becomes a title with its provider;
an imported placeholder has none and is unavailable; "show"/"tv" are series.
Tests: 3 in `watch-core`.

**P2 — the Watch library (ours).** `crates/watch/src/library.rs`:
`Library { titles: Vec<Title> }` built from the same rails the current pipeline
uses (provider rails, catalogs, the personal shelf, placeholders), so one place
knows how a title comes to exist. Acceptance: the library's titles equal the
`MovieItem` set the old path produces (same ids, same order, same availability);
a placeholder is `available: false`; no network in tests.

**P3 — the screens read titles (ours).** `app/src/fork/watch/sources.rs` and the
film page consult `Title` (`id`, `kind`, `year`, `available`, `rails`) instead of
`extra` keys. Acceptance: the film page, marks, trailer hint, personal flags and
the watchlist rail behave identically; `sources.rs` no longer reads `extra`.

**P4 — the writer emits from titles (ours).** `crates/watch-state/src/collection.rs`
takes `Title`s and writes the library JSON (still the shapes upstream reads:
`runner`, `app_name`, `is_installed`, `extra.stremioType`). The adapter gets
narrower; `installed` is documented in code as "available, for the shared
renderer". Acceptance: the app renders identically for a normal title and for a
placeholder; the JSON keeps its keys so a downgrade still reads it.

**P5 — only if upstream moves.** If upstream ever ships a library-item trait or a
Watch space of its own, adopt it and delete the adapter. Not planned.

## 6. What we deliberately do not do

- **Rewrite the renderer.** `card`/`tile`/`carousel`/`spotlight` stay upstream's.
  They keep eating `CollectionGame`-shaped data produced by our adapter, so themes,
  widgets, layouts, badges and focus keep arriving from upstream.
- **Rename upstream's crates or types.** `CollectionGame` remains the *view* type;
  it simply stops being where Watch *stores* its data.
- **Change what `installed` means for games.** The repurpose is confined to the
  watch library writer, which is ours.
- **Move the film page onto upstream.** It is already ours and stays there.

## 7. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Shared-file edits accumulate (grey conditions, `CardVm.imported`) | Keep them to the adapter; record each in `docs/FORK.md`; prefer reading `extra` over adding fields when a revert is cheaper |
| Two models drift (a `MovieItem` and a `Title` disagreeing) | P2's acceptance test compares them; the adapter is one function |
| Themes stop seeing what they expect | P4 keeps the JSON keys and the `CollectionGame` fields identical |
| The film page and the tiles disagree about "available" | `available` is derived once (P2) and mapped once (P4) |
| Upstream merges rename a field we adapt to | The adapter is the single place to fix; no domain code changes |

## 8. Gates

`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, the i18n coverage gate, and `cargo check -p heronite`
after each phase. A phase that changes the shared column must also say why in
`docs/FORK.md`.

## 9. Open questions for review

1. Is `available` the right noun, or should it be `playable` / `sourced`? It is the
   one word the screens, the import and the filter (later) will share.
2. Should `Title::rails` be catalog ids only, or also the kind of rail (home rail,
   collection, personal)? Today it is `catalog` only.
3. Should a title keep the ids the providers use between themselves (`simkl`,
   `tmdb`) or only the canonical one? Today: only the canonical id, with the
   provider's own id irrelevant once matched.
4. Is the `CardVm.imported` field worth keeping, or should the five grey checks
   read `g.game.extra` directly to keep the shared diff at zero new fields?

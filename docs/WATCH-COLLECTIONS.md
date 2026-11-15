# Watch — Collections, watchlist and favourites (design)

Status: **spec, revised after a design review.** Fork-owned. Read alongside
`docs/WATCH.md` (the contracts) and `docs/WATCH-CRATES.md` (the crates). This
document owns the vocabulary and the rules; the implementation must not widen them
without updating it.

> **Revision 2026-11-15, after review.** Three things changed the shape: a personal
> rail draws from its own snapshot (the projection cannot draw a title it does not
> hold), the collection contract carries the source and the media kind (a rule is
> only valid for the source that declared it), and rule rails get a real
> replacement primitive and a real freshness key instead of borrowing the catalogue
> epoch. Findings from the review are folded in below; §12 lists them.

## 1. The words, and who owns each

| Thing | What it is | Owner | Needs |
|---|---|---|---|
| **Continue watching** | where the person stopped | Stremio, the media provider (`CONTINUE_WATCHING` rail) | the linked account |
| **Watchlist** | titles the person means to watch | **local** (TMDB account later, §9) | nothing |
| **Favourites** | titles the person likes | **local** (TMDB account later) | nothing |
| **Collection** | a named rail of titles: a *rule* ("Top rated", "Popular") or a *fixed list* (a franchise's parts) | **a provider** declares it, the person turns it on | that provider's key, when it has one |

TMDB's `/collection/{id}` is a **franchise** — a fixed list, not something a person
curates. TMDB's *lists*, *watchlist* and *favourites* are account state and need a
session (§9). "Most played" does not exist anywhere in our vocabulary: a catalogue
sorts by `Popular | Rating | Newest | Title`, so the rail is called **Popular**.

## 2. What replaces the Favourites tab

The Watch space gets a tab with source `"collections"` (name **Collections**). A
theme that carries a `favourites` tab has it replaced — source *and* layout
(`presets::cinema` carries one as `Layout::Cinema`), so the projection forces
`Layout::Shelves`; a theme with none gets the tab added. The theme's `favourites`
**shelf** is dropped for Watch (nothing ever sets `favourite` on a Watch title, so
it draws nothing).

Rails, top to bottom:

1. **Continue watching** — the Stremio provider's rail, as today.
2. **Watchlist** — the local list.
3. **Favourites** — the local list.
4. **Enabled provider collections** — one rail each, in settings order.

Rules:

- A rail with nothing in it is hidden, not drawn empty.
- A rail whose provider failed draws **one fixed line** ("Could not load"); the
  reason goes to the log only, because a transport URL can carry a debrid key.
- The tab with nothing at all shows one hint: where to add a title, where to turn a
  collection on.
- Continue watching and the local rails work with no network at all: they draw from
  the provider's rail and from `watch-personal.json`, never from the projection.

## 3. The model (`heronite-watch-core`)

```rust
/// A title as a personal rail remembers it: the card's fields and the key the rest
/// of Watch resolves it by. Deliberately `MovieItem` — one card model, not two.
pub type PersonalTitle = MovieItem;           // provider, kind, id, name, year, poster, background, …

/// What the person owns: one file, one struct.
pub struct PersonalLibrary {
    pub watchlist: Vec<MovieItem>,
    pub favourites: Vec<MovieItem>,
    pub lists: Vec<PersonalList>,             // reserved: a local list is written the same way
}

/// A local list. `remote` is reserved for TMDB's list id, so the account slice
/// does not force a migration.
pub struct PersonalList { pub id: String, pub name: String, pub items: Vec<MovieItem>, pub remote: Option<String> }

/// A rail a provider declares for the Collections tab.
pub struct CollectionDescriptor {
    pub id: String, pub name: String,
    /// `movie` or `series`: a rule answers for one kind, so a provider wanting both
    /// declares two.
    pub kind: String,
    pub kind_of: CollectionKind,
}

pub enum CollectionKind {
    /// A saved query, **with its source**: two sources may name a catalog alike, and
    /// a stored rule is only valid for the source that declared it (a genre name is
    /// "Sci-Fi" in Cinemeta and "Science Fiction" in TMDB).
    Rule { source: String, request: CatalogRequest },
    /// A franchise's parts, or any fixed list a provider hands over.
    Items(Vec<MovieItem>),
}

pub trait WatchCollectionSource: Send + Sync {
    fn descriptor(&self) -> &CollectionSourceDescriptor;                 // id, name — Settings lists it
    /// `CatalogCtx`, not `MetaCtx`: listing and resolving both need the provider's
    /// key, and resolving needs the id cache.
    fn collections(&self, ctx: &CatalogCtx<'_>) -> Result<Vec<CollectionDescriptor>>;
}
```

**Identity.** A personal title is keyed `provider:kind:id`. The projection's own key
is `provider:id` (`app_name`) and cannot express kind, so nothing in a personal rail
may lean on it.

**Ordering.** Insertion order; `add` appends; a title already present is a no-op
whose snapshot is refreshed in place. Provider rails are ordered by the settings
list; reordering UI is out of scope for the first pass (the list is hand-editable).

## 4. Storage (`heronite-watch-state`)

- `<config>/watch-personal.json` — the `PersonalLibrary`. **User data**: never in the
  cache, never rewritten by a library sync, hand-editable, ready for backups and
  device sync.
  - Writes go through `json_store` (already atomic: temp file + rename).
  - **Every struct in the file carries `#[serde(flatten)] extra: Map<String, Value>`**
    — that, and only that, is what "unknown fields are kept" means. An unreadable
    file is the default value and is warned about **once per path** (a `OnceLock`),
    because the file is read on every refresh.
- `<config>/watch-settings.json` gains `collections: Vec<String>` — the enabled
  provider collection ids, in rail order.
- `<cache>/watch-collections/<provider>-<kind>-<id>.json` — one disposable file per
  rule rail: `{ "fetched_at": …, "items": [...] }`. It is the rail's only freshness
  key (§5); it is deleted when the rail is turned off.
- **One writer for the projection.** `write_root` writes four files in sequence and
  `append`/`replace` are read-modify-write; today two writers exist and a rule rail
  would be a third. The facade serialises every projection write behind one
  process-wide mutex.

## 5. Resolution and freshness

A **rule** rail is resolved like a tab page: `catalog::page(source, request)` for
page 0, then merged into the projection so every card can open. Two corrections to
the first draft:

- The projection gets a **per-rail primitive**: `collection::replace_feed(root,
  feed_key, items)` plus `collection::remove_feed(root, feed_key)`, and the key is
  `collection:<kind>:<source>:<catalog>-<sort>[;filters]`. `replace(kind, …)` (which
  drops by the `"<kind>:"` prefix) is **not** usable for this: it would take the
  tabs' pages with it, and a `collection:` key never matches it, so rails could only
  grow.
- Freshness lives in the rail's cache file, not in the app's catalogue epoch (that
  is a session counter, never persisted, and shared by an unrelated setting): a rail
  is refetched when its file is older than an hour, when it is missing, or when the
  collections setting changed. Turning a rail off calls `remove_feed`.

A personal rail needs none of this: it renders from the snapshot.

## 6. The tab page (`app/src/collection/console/watch_collections.rs`, fork-owned)

Dispatched from the console's Shelves path — **three arms, not one**:
`render_shelves` → here, `shelf_pad` → here (navigation, activation), `shelves_hints`
→ here. That is a new deviation row (**F-015**) covering
`app/src/collection/console/layouts/{mod,shelves}.rs`, listed in `docs/FORK.md` and
in `docs/WATCH.md`'s editable-upstream list. The page never calls `console_shelves`:
`console_in_source` has no `"collections"` case and falls through to "everything".

The page draws a small enum, named `Shelf` so it cannot be confused with the
provider's `Rail`:

```rust
pub enum Shelf { Continue(…), Personal { title: String, items: Vec<MovieItem> },
                 Provider { source: String, descriptor: CollectionDescriptor, items: Vec<MovieItem> },
                 Failed { title: String } }
```

**Drawing and opening.** Cards come from the item's own art (`ImageRef::Url` +
`images.get_or_request`, as `episode_still` already does), not from the projection's
cover chains. Opening a title that the projection does not hold works because the
film page and the stream path are widened (both fork-owned): `watch_ensure_details`
and `watch_ensure_streams` take a `TitleRef` plus what the snapshot shows, and the
header/`watched_line` tolerate a missing `CollectionGame` (no minutes, no
last-watched). Without that, a watchlist entry for a title the last refresh dropped
would open as "This title is gone".

## 7. Actions

On the **film page**, rows 2 and 3 of the list are toggles (the row model already
supports them: `Row::toggle`/`Value::Toggle`):

- **Add to watch** / **In watchlist**
- **Favourite** / **In favourites**

`A` flips; the label swap *is* the state. Both are local writes: instant, offline.

**Removal** is not `X`: on the Shelves layout `X` is already the second of A's
actions, and on the film page it is Play with a hint that says so. Inside the
Collections tab, removal comes from A's second action opening the existing
confirmation dialog, and the tab's hint bar says which key that is.

## 8. Settings

Settings → Watch → **Collections**: one toggle per provider-declared collection, in
rail order. Two dependencies the UI must state:

- A collection can only be resolved while the **catalog source that answers its
  rule** is enabled (`catalogSources`); a rail whose source is off is absent, and
  the row says why.
- A provider that cannot list its collections (no key) makes listing *fail*, not
  return nothing: the row shows "add the key", not an empty list.

## 9. Deferred, each a slice of its own

1. **TMDB account** — `/authentication/token/new` → approve at
   `themoviedb.org/authenticate/{token}` → `/authentication/session/new` →
   `session_id`, plus `/account`; writes `POST /account/{id}/watchlist`,
   `/favorite`, `/list`. Deferred by decision: requiring a TMDB account for "Add to
   watch" would make the button dead for most people. The flow needs an approval
   screen (the app already owns a WebKit login window) and a new secret.
   **Sync policy, corrected:** the local file is the offline truth; each write marks
   the title **dirty with a time**; a push resolves the IMDb id to TMDB's media id
   (`/find`, the reverse of the metadata source's use) and titles TMDB does not know
   stay local; a pull never clears the local file; a **removal writes a tombstone**
   (`removed_at`) so the next pull cannot resurrect it, and a tombstone is dropped
   once a pull confirms the absence. With no remote modification time available, the
   order is: push dirty first, then pull; a dirty title the pull lacks is re-pushed.
2. **TMDB franchises** — `belongs_to_collection` from the details we already fetch,
   then `/collection/{id}` for the other parts, declared as `CollectionKind::Items`.
3. **Local named lists** — `lists` exists in the model from the start, with `remote`
   reserved, so "Add to a list…" and a "Your lists" rail need no migration.

## 10. Failure modes

| Situation | Behaviour |
|---|---|
| No provider key (TMDB) | its listing fails and is caught: the Settings row says where the key goes, the rail is absent |
| A provider errors while a rail is resolved | that rail draws the fixed line; the rest draw |
| Offline | continue watching and local rails work; rule rails keep their cached file, or are absent |
| A personal title left the projection | the rail draws the snapshot; opening it asks the details layer by id |
| Library refreshed | `watch-personal.json` is untouched; only the projection changes |
| Two writers on the projection | impossible: the facade owns the only lock |

The facade's rule that a **failed provider aborts the library refresh and keeps the
last good library** is unchanged for the home; the tab resolves its own rails and
isolates their failures. That difference is deliberate and stated here.

## 11. Tests

- `watch-core`: `PersonalLibrary` round-trip; `add` idempotence and insertion order;
  `remove`; the key is `provider:kind:id`.
- `watch-state`: `watch-personal.json` round-trip **with unknown fields kept**; a
  corrupt file is the default and is warned once; the rail cache file's `fetched_at`;
  `replace_feed`/`remove_feed` touch only their key (a tab page and another rail
  survive).
- `watch-catalogs`/`watch-tmdb`: descriptors from a fixture (id, name, kind, the
  rule's source and media kind); a rule resolved through a fake transport; a source
  without a key making listing *fail*.
- `heronite-watch::collections`: `shelves(app)` order (continue watching, local,
  then enabled providers), empty rails hidden, a failed provider becoming
  `Shelf::Failed` without touching the others, a rail whose catalog source is off
  absent.
- App: the page draws the shelves in order and hides empties; a failure draws one
  line; removal goes through the dialog; the film page's toggles flip and persist;
  a snapshot title with no projection record still opens.

## 12. What the review changed

1. Personal rails draw from the snapshot and open through a widened film page — the
   projection cannot draw or open a title it does not hold. Rule rails still go into
   the projection, and `replace_feed` is what keeps them from growing forever.
2. The contract carries the source, the media kind and a `CatalogCtx`; a provider
   without a key fails its listing rather than declaring nothing.
3. Freshness is a per-rail file with a time, not the app's catalogue epoch; one
   writer owns the projection; `PersonalTitle` *is* `MovieItem`; the page enum is
   `Shelf`; the seam is three arms and a new F-015; `X` keeps its existing meaning.

# The Watch page (console mode)

Status: decided; Phase 0 landed, Phases 1–5 follow.
Owner: the fork. Register rows: `F-012`, `F-022`. Seam table:
`app/src/fork/seams.rs` (machine-checked).

Where the work stands, and how to continue it: `docs/WATCH-PAGE-STATUS.md`.

This is the design record for the console screen a person opens when they press
A on a film or a series. It replaces the earlier note that the film page "is
already ours and stays where it is" with a shape that survives upstream
rewrites and still offers the theme's page styles.

## 1. The problem

A Watch title is not a game. It has no install, status, playtime, save folder,
screenshots, journal, achievements or How Long To Beat; it has episodes, a
season picker, streams, rails of similar titles, and three marks (watched,
watch list, favourite).

Upstream's console detail page is a **game** page with five styles, chosen per
theme by `theme.game_page` (Classic, Hero, Tabs, Sheet, Showcase). It is also
**code-driven**, not data-driven, which is what decides this whole design:

- the Spotlight/Showcase page precomputes its layout in `Shape`/`Geo` over a
  closed `Zone` enum — a new section means new heights and a new focus zone;
- `FocusVm` and its only constructor are `pub(super)`, so a fork module cannot
  build one;
- `Row` cannot carry a still image, so an episode card cannot be a row;
- four of the five layouts treat `Outcome::Changed` as Activate, so a season
  picker moved with ◀ ▶ would start playing the picked episode;
- the Sheet filters its rows through a six-id allow-list, which drops every
  Watch row.

Hosting Watch inside those five layouts therefore is not a seam: it is a
rewrite of each, in the files upstream rewrites most. Measured: that shape
touches 11 → at least 14 upstream files, and moves the fork's lines *into* the
layout bodies. It was rejected.

## 2. The decision

1. **The Watch detail page is fork-owned, as one subsystem**, not one 1,600-line
   file: a host that answers the shell, an engine that owns the page's own view
   state and geometry, a section per part of the page, and a facts adapter for
   the wording and the metadata. Film ships first with output identical to
   today's page.
2. **Upstream's layouts are never edited for the page.** Not a branch, not a
   call into the fork. Enforced by `app/src/fork/seams.rs` tests, not by
   convention.
3. **The shell's questions are answered at one seam.** The shell asks seven
   things of the page (render, floating layer, pad, hints, scrim, whether it
   scrolls text, whether it keeps the tab bar). Today each is an `if
   space == Watch` in `console/game/mod.rs`. The target is a fork-owned page
   value in the console's state, consulted by three sites; the remaining
   answers read that value instead of a space check.
4. **The space's page configuration lives in `console-watch.json`**, as a typed
   fork-owned block in `ConsolePrefs.extra`. It is not a field of upstream's
   shared `ConsoleTheme`: adding one would be a new deviation in the crate the
   fork is trying to leave alone, and `present()` replaces the theme wholesale,
   so such a field would be lost every time a theme is taken.
5. **The page owns view state only.** `watch_playback.rs` keeps the data
   (details, streams, the rail keep/load machine) and the desktop detail panel
   keeps rendering from it. The page does not become a second data owner.
6. **Nothing is proposed upstream.** The fork does not send upstream an issue, a
   pull request or a branch; its upstream contact happens only when the
   maintainer asks for it (`docs/FORK.md`). The page is the fork's own and is
   built to work against upstream as it is.

### Rejected shapes

| Shape | Why not |
|---|---|
| **A** — keep today's single file as the page | It cannot offer the theme's page styles, duplicates chrome, and its page logic is entangled with layout math and nav in one 1,600-line file |
| **B** — host Watch inside the five upstream layouts | Measured larger (11 → ≥14 upstream files), lands in the bodies upstream rewrites every sync, and needs a new `Zone`, a new allow-list, a `Changed`-vs-Activate fix and an image-capable row just to start |
| **C** — fork-owned page subsystem behind one seam | **chosen** |

## 3. Where it lives, and why not `app/src/fork`

`app/src/collection/console/watch_page/`. The page needs `CollectionView`'s
private fields, `console.game`'s `pub(super)` state, and helpers such as
`console_tile`, `cover`, `facts_table` and `STAR`. Rust's privacy makes those
unreachable from `crate::fork`: `mod console;` and `mod game;` are private,
`CollectionView`'s fields carry no visibility marker, and no `impl
CollectionView` exists under `app/src/fork` today. Putting the page there would
force a dozen `pub(crate)` widenings in the files this design exists to leave
alone.

So the page is **fork-owned files inside an upstream module**: the same
arrangement as `app/src/collection/watch_playback.rs`, which `docs/FORK.md`
already lists as an addition, not a deviation. The directory is added to that
list in the same commit that creates it.

## 4. The seam contract

Upstream keeps: the five page styles for games, every layout, every home
screen, and the desktop panels. The fork's upstream lines are the table in
`app/src/fork/seams.rs`, whose tests enforce it:

- `every_seam_is_in_the_register` — a seam file is in the register's own list,
  and every row it cites exists;
- `the_register_lists_no_seam_the_table_lacks` — the register lists nothing the
  table does not carry;
- `every_trailer_is_in_the_table` — every file the tree marks with a
  `Fork-Seam` trailer has a row, so the table cannot shrink behind the markers;
- `every_seam_site_still_exists` — the fork's lines are still in the files (a
  rename on either side fails here, not at the next reconciliation);
- `upstream_layouts_never_dispatch_the_fork_page` — every file of upstream's
  detail page except its dispatch file (`console/game/mod.rs`) has no branch on
  the space and no page entry point, and nothing under `console/layouts/**`
  calls the page. Per-space *wording* in the layouts stays allowed under
  `F-016`/`F-021`;
- `the_film_page_is_referenced_only_at_its_seam` — only the page and its
  dispatch name the page's entry points.

## 5. Identity

The page carries the space's own vocabulary: **Watched**, **Last watched**,
**Not watched yet**, **Season 2**, **Episode 4**, **Similar**, **Community**,
**Watch later**, **Play trailer**. Those strings live in the page and in the
shared helpers that already exist (`app/src/collection/console/hero.rs`:
`watched_or_played`, `played_line`, `console_facts`, `console_stats`), and the
i18n coverage gate keeps the four locales honest.

We do not rename upstream's `GamePage`, and Games' setting stays Games'. A fork
style may carry one of upstream's style names only when it genuinely is that
arrangement; **Film** is the fork's own, and it is the default.

## 6. Phases and gates

| Phase | What lands | Gate |
|---|---|---|
| 0 | this record; `F-012` re-baselined, `F-022` added; the seam table and its four tests; the reversed decisions in `docs/WATCH-TITLES.md` | the workspace gate, with the new tests |
| 1 | the watch-shaped facts view model, and the five space-aware call sites routed through it; the film-shaped bug fixes (§8) | behaviour identical; i18n gate |
| 2 | engine + sections; the page file becomes the host; six new tests before any monolith code is deleted | film output identical (the acceptance, not "looks fine") |
| 3 | page styles, Film first; the parity policy for any other style | per-style screenshot check against the committed reference |
| 4 | page configuration in `console-watch.json`; the Watch row with reset/diff; the isolation test | Games prefs byte-identical after every Watch edit |
| 5 | the leftover scaffolding removed | the seam table shrinks or stays; never grows |

Every phase runs `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace`, and is reviewed before the next one starts.

## 7. What we accept

- **Watch does not inherit upstream's page work.** Upstream's page improvements
  reach Games only. That is the trade for never editing its layouts.
- **Three renderers exist**: upstream's page (Games), this page (Watch), and the
  desktop detail panel. The panel is independent and stays untouched; the other
  two share no code by design.
- **New upstream surfaces still cost a seam.** One sync brought four (an
  achievements screen, a launch screen, a session card, a now-playing screen),
  and each needed a hook. The budget is measured in **seam sites per file**, not
  in lines, and the facts adapter exists so a new surface can ask the space
  instead of being hooked.
- **Styles mirror a vocabulary, not pixels.** A Watch style is not a copy of
  upstream's implementation; it is the same arrangement, drawn by the page.

## 8. Known film-shaped defects this work fixes (Phase 1)

Found while reviewing the page and independent of the refactor:

1. a catalog film reports `installed = true` (`crates/watch-state/src/collection.rs`),
   so upstream's surfaces print "Store · Installed" and offer an install-folder
   link for a film;
2. `classic_facts` filters the facts table on the literal labels "Last Played"
   and "Playtime" (`console/game/mod.rs`), so renaming those rows for Watch
   changes which facts Classic shows;
3. a rail's cards, its focus count and its pad routes were two sources of truth
   (a drawn filter on one side, `details.rail(rail).len()` on the other); they
   are one list now, so a title the library does not hold yet cannot leave a gap
   or a press that goes nowhere. The empty box the earlier code could draw was
   already unreachable behind the "missing" guard — the disagreement was the
   real risk;
4. `watch_rows`, `watch_press_row` and `watch_page_pad` have no tests today —
   the only net over the dispatch this design creates.

## 9. Deliberately open

- which styles Watch ships besides Film (Phase 3 decides, with the parity
  check);
- whether the engine absorbs `watch_playback.rs`'s view state (Phase 2 measures
  the two owners and keeps data state where it is either way);
- the parity check's form for styles (decided: a committed screenshot compared by
  hand, §10).

## 10. The upstream parity policy

Upstream's page is code, not a template: its five styles are hand-written
drawing routines whose geometry (`Shape`/`Zone`/`Geo`, computed in `layouts/`) is
closed and whose facts model is private. Hosting Watch inside them was measured at
11 → ≥14 upstream files, with the fork's lines landing in the bodies upstream
rewrites most often, and it would have hit real defects on day one: four styles
re-dispatch a changed value through the games' press handler, where a season id
matches no row and is dropped silently (a season picker would simply do nothing),
the Sheet's six-id allow-list drops every film row, Hero has no list at all, and a
`Row` can carry a still only as a path on disk — not the URL a catalogue hands
out. So Watch draws its own arrangement and reuses upstream's **components** — the
list, the tiles, the theme tokens, the focus and scroll helpers — which is where
most of upstream's polish lands.

That trade is only sound if it is re-measured, not believed. This section is the
rule for doing that.

**Naming.** A fork style may carry one of upstream's detail-page names (Hero,
Tabs, Sheet, Showcase) **only** when a side-by-side shows the same arrangement:
the same blocks in the same places, the same focus order. Otherwise the style
gets its own name. A person is never promised a thing the page does not do, and
`style.rs` says the same in code.

**The parity check.** Every style after Film ships with a committed reference
screenshot, taken on a known window size with the Film theme, and the style is
compared to it by hand on each release — not by a unit test, because the thing
being checked is a picture. A style whose picture changes without a matching
intent in the commit is a regression, exactly like a failing test.

**The revisit rule (the numbers, not the feeling).** After every upstream sync,
count how many of `app/src/collection/console/game/*.rs` and
`app/src/collection/console/layouts/**` changed, and write the count into the
sync log in `docs/WATCH-PAGE-STATUS.md`. The decision to host Watch inside
upstream's layouts is re-opened when the detail-page files go **two consecutive
syncs without a change** — at that point re-grafting film logic into them stops
being a treadmill and the trade flips. Until then, staying out is cheaper, and
the log is the evidence either way.

**The degradation contract.** If the fork ever offers "an upstream style for
Watch" as an option, it is labelled as such and it degrades safely: no episodes,
no streams, no rails; a season press may never launch anything; the row filter may
never empty the page. Those three are the defects found in review, and each gets
a test before the option ships.

**Who owns what.** Film is the fork's and is always available. Upstream's styles
serve games only unless the person explicitly opts into the contract above. The
fork never *dispatches* its page from upstream's detail-page layouts —
`app/src/fork/seams.rs` fails the build if one tries. (The layouts may still word
things per space under F-016/F-021; that is not a dispatch.)

**No upstream proposal.** The fork does not offer this screen, or a hook for it,
to upstream: its upstream contact happens only when the maintainer asks for it
(`docs/FORK.md`). If that ever changes, the interface worth proposing is the
seven questions the shell already asks — a trait with a default-empty registry —
but nothing is prepared or sent on the fork's own initiative.

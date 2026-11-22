# The Watch page — status and handover

Working record for the Watch page refactor. `docs/WATCH-PAGE.md` is the decision
(why the shape is what it is); this file is **where the work stands and how to
continue it**. Read both before touching code.

Branch: `feat/watch-page-adr` (cut from `fork/main`). Latest commit at the time
of writing: `97c93b76`. Working tree clean, gate green.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p heronite watch_page          # 23 tests
cargo test -p heronite watch_facts         # 4 tests
cargo test -p heronite fork::seams         # 8 guards
cargo test -p heronite fork::watch::prefs  # 3 tests
```

## Where the work stands

| Phase | State | Commits |
|---|---|---|
| 0 — design record, register re-baseline, seam table | **done, reviewed** | `47733049` |
| 1 — facts view model + film-shaped fixes | **done, reviewed** | `f998c79c`, `4a35719f`, `27437abf`, `a1901289` |
| 2 — engine + sections | **done, reviewed** | `8fa89b8d`, `b9406d5e`, `711d9c6c`, `97c93b76`, `5018e018`, `70894e35`, `9d1f5798` |
| 3 — page styles (Film first) | not started | — |
| 4 — page config in `console-watch.json` | **partly** (config + isolation + the page reads it) | `8221227d`, `584e7de3` |
| 5 — retire the scaffolding | **partly** (style vocabulary collapsed, the marks section moved; the rails renderer remains) | `44445aa2`, `c768056f` |

Phase 2's six gates are all green and runnable:
`cargo test -p heronite watch_page` (24), `fork::seams` (8),
`fork::watch::prefs` (3), `cargo test -p heronite-i18n --test coverage` (2).

One deliberate deviation to keep: `episode_name`/`episode_label`/`episode_desc`
stay `pub(in …::watch_page)` because `mod.rs`'s test module still exercises them
through the `series()` and `Selection` fixtures, which belong to the page, not to
the section. Moving those tests would duplicate the fixture; the functions are
private otherwise. The reviewer's other items (the `names`/`fact_label`
visibility, the mark-count doc) are done.

### What Phase 0 landed

- `docs/WATCH-PAGE.md` — the decision: the Watch page stays fork-owned as one
  subsystem under `app/src/collection/console/watch_page*` (an addition, not a
  deviation); upstream's detail-page layouts are never edited for it; the space's
  page config will live in `ConsolePrefs.extra` inside `console-watch.json`, not
  in upstream's `ConsoleTheme`.
- `app/src/fork/seams.rs` — **test-only** (`#[cfg(test)]`) table of 25 seams and
  eight guards. It is the source of truth for the fork's upstream surface:
  - `every_seam_is_in_the_register` / `the_register_lists_no_seam_the_table_lacks`
    — the table and the paragraph in `docs/FORK.md` (search for
    `The machine-checked list of those seams is`) agree in both directions;
  - `every_trailer_is_in_the_table` — every `.rs` under `app/src` and `crates`
    with a `// Fork-Seam:` line must have a row;
  - `every_seam_site_still_exists` — each row's tokens are still in its file;
  - `the_dispatch_seam_names_every_page_entry_point` and
    `the_dispatch_seam_answers_seven_shell_questions` — the page's entry points
    and the seven `self.space == super::super::Space::Watch` answers live in
    `console/game/mod.rs`;
  - `upstream_layouts_never_dispatch_the_fork_page` /
    `the_film_page_is_referenced_only_at_its_seam` — no layout body calls the
    page, and only the page and its dispatch name its entry points.
- `docs/FORK.md` — F-012 re-baselined; **F-022 added** (`main.rs`'s `mod fork;`,
  `focus_left.rs`, `context_menu.rs`, edited since F-010 without a row); the
  containment list names the page and `watch_facts.rs` as additions; the register
  is measured against **`upstream/main`** because `main` lags one sync.
- `docs/WATCH-TITLES.md` — the reversed decisions (the film page stays ours as a
  subsystem; the budget is seam sites per file, not lines).

### What Phase 1 landed

- `app/src/collection/watch_facts.rs` (**new, fork-owned**): `watched_time`,
  `time_label` ("{time} watched"/"played"), `when_label` (Last watched / Not
  watched yet), `played_line`, `facts_line`, `relabel` (detail rows become "Last
  watched"/"Watched" for a film), `library_line` (games only), `store_line` (a
  film names its catalogues, not "Local"). Four tests.
- Routing: `console/hero.rs` (`played_time`, `watched_or_played`, `played_line`,
  `console_facts`), `collection/data.rs::focus_vm` (new `space` parameter +
  `relabel`), `collection/actions.rs` (passes `self.space`),
  `console/game/mod.rs::classic_facts` (filters by `DetailField`, not by label),
  `console/render_focus.rs` (same), `layouts/spotlight/mod.rs` (`store_line`; no
  folder link for a space that cannot install), `console/watch_page` (a rail's
  render, focus count and pad routes read one `rail_cards` list).
- Games output is **byte-identical**: a reviewer diffed the moved bodies against
  `git show f998c79c^:app/src/collection/console/hero.rs`.

### What Phase 2 has landed (incomplete)

- `console/watch_page.rs` → `console/watch_page/mod.rs` (the page is now a
  module; `mod` paths are unchanged for callers).
- `console/watch_page/engine.rs` (new): `marks_len`, `mark_at`,
  `focused_episode`, `rail_route`, `mark_route` — pure routing, no window, no
  rendering — with three tests; `mod.rs` imports them so a click and a press
  still take the same route.
- `fork/watch/prefs.rs`: the isolation test (`console-watch.json` never rewrites
  `console.json`).
- `engine.rs`: the season-vs-episode test (a season row names no episode).
- `sections/` exists: `sections/mod.rs` (the map) and `sections/episode.rs`
  (`episode_name`, `episode_label`, `episode_desc`, `episode_row`,
  `episode_card`, moved verbatim; visibility
  `pub(in crate::collection::console::watch_page)` — `pub(super)` stops at
  `sections`, which does not compile from `mod.rs`).

## Phase 2 — the remaining slice (start here)

Create `app/src/collection/console/watch_page/sections/` and move the renderers
out of `mod.rs` **unchanged** (output identical is the acceptance). Suggested
split, one file per concern:

| File | Moves |
|---|---|
| `sections/episode.rs` | **done** — `episode_name`, `episode_label`, `episode_desc`, `episode_row`, `episode_card` |
| `sections/details.rs` | **done** — `facts_line`, `title_status`, `watched_line`, `names`, `detail_lines`, `fact_label` (`DetailLine` stayed in `mod.rs`; the section reaches it through `super`, so its fields were not widened) |
| `sections/rails.rs` | `rail_of`, `panel_room`, `columns_in`, `rail_columns`, `rail_line_width` (free functions — they take `PAGE_PAD`/`LEFT_W`/`COLUMN_GAP`/`PANEL_PAD`/`RAIL_GAP` from `mod.rs` and `super::layouts::body_width`, which becomes `super::super::super::layouts::body_width`) and then `watch_rail` itself, which needs an `impl CollectionView` block in the section (legal: it is a descendant of `collection`) |
| `sections/marks.rs` | `watch_marks`, likewise an `impl CollectionView` block |

Learned while moving the first two: `pub(super)` stops at `sections/`, so a
section's items need `pub(in crate::collection::console::watch_page)`; moving a
function out of `mod.rs` turns any import only it used (`trn`, `Fact`, gpui's
prelude) into an unused-import error under `-D warnings` — remove them in the
same commit, and give the one test that still needs `trn` its own import.

Rules while moving:

1. **Move each function's tests with it** (`docs/CODE-STYLE.md`), keeping
   `super::*` working. The 23 `watch_page` tests are the acceptance gate: they
   must all pass after every move, and `git diff` must show no changed behaviour.
2. Keep `pub(super)` visibility; `mod.rs` re-exports what the render shell and
   the pad handler use.
3. Any new fork line in an **upstream** file needs either a row in
   `app/src/fork/seams.rs` with a whole-line token, or a `// Fork-Seam: <id>`
   comment (the trailer test enforces the second).
4. After the moves, the six Phase 2 gates are complete: season-vs-episode (done),
   marks press (`a_press_on_a_mark_writes_the_flag_it_draws`), rails press
   (`watch_open_related` — still needs a view; consider extracting its decision
   into `engine.rs` as a pure function so it can be tested), dispatch (the seam
   guards), prefs isolation (done), i18n (`cargo test -p heronite-i18n`).

## Phase 4 — what is left (start here)

Landed: `app/src/fork/watch/page_config.rs` (the choice in `ConsolePrefs.extra`,
read leniently, six tests including the Games-file isolation), and
`watch_page/style.rs::PageStyle::current(&app_paths)` now reads it, so the page
draws what the space chose (Film until something writes a choice).

Left — the settings row, in **fork-owned files only**
(`app/src/fork/watch/settings/mod.rs`), so no new seam is needed:

1. `console_rows` (line ~1455) gains, between the theme row and the reset row:
   `Row::choice("watch-page-style", tr!("Film page"), names, index)` where
   `names` is `page_config::PageStyle::ALL` labelled and `index` comes from
   `page_config::read(&themes::prefs_for(&d.ctx.app, Space::Watch))`.
2. `console_apply` (line ~1670) gains two arms:
   `Outcome::Changed(id, Value::Choice { index, .. })` for that row, and
   `Outcome::Activate("watch-page-style-reset")`. Both write through
   `crate::console::themes::update_for(&d.ctx.app, Space::Watch, |p| page_config::write(p, config))`
   — never touching the Games prefs — and `cx.notify()`.
3. A reset row (`Row::action("watch-page-style-reset", …)`) that writes the
   default; that is the per-row "reset" the review asked for (with one style
   today, a per-row *diff* has nothing to compare, so the reset is the honest
   half of it).
4. When the row lands, drop `#[cfg(test)]` from `page_config::{write, PageStyle::ALL, PageStyle::as_str}`
   (they are gated only because nothing wrote the choice yet) and extend the
   isolation test to assert the choice survives a save/load of the space's file.

## Phase 5 — what it means

Retire what the split left behind in `watch_page/mod.rs` (the render shell keeps
only what a style needs), and open a branch off `upstream/main` offering the
generic space/section **hook** (`§10`): an interface, not the fork's code. Never
a dependency.


- **Phase 3 (styles).** Add `PageStyle` in the page module (`Film` first, output
  identical to today). A fork style may carry an upstream style's name only if it
  genuinely is that arrangement. Each style gets a screenshot check against a
  committed reference (name the check before writing the second style).
- **Phase 4 (config).** A typed fork config in `ConsolePrefs.extra` inside
  `console-watch.json` — **not** a field of `ConsoleTheme` (that would be a new
  deviation in `crates/data` and `present()` replaces the theme wholesale, so the
  field would be lost every theme take). Requirements: normalise it in **both**
  `present(Watch)` and `validate(Watch)`, or every install reads "Customised";
  clear it in `validate(Games)`; give the fork row a per-row reset/diff (the
  editor's `opt()`/`changes.rs` resolve by row id — see `theme_rows/options.rs`
  and `changes.rs`); the preview keys on the literal id `"game_page"`
  (`pages/settings/console_mode/mod.rs`), so the new row needs its own preview or
  none.
- **Phase 5.** Remove the scaffolding left in `mod.rs`: the rails renderer and
  its geometry helpers are the last of it. Nothing is proposed upstream —
  upstream contact happens only when the maintainer asks (`docs/FORK.md`).

## Conventions and gotchas (learned the hard way)

- **The host's `complete_step` refuses review evidence** unless a review run is
  recorded in the same turn, and it refused repeatedly even then. Phase 1 was
  signed off with the review verdicts recorded as a `manual` evidence item. Do
  not read that gate as a code problem; keep running the review and record its
  verdict.
- **Reviews are per phase** and use `run_skill("review", …)` (or the
  `skill:review` capability). The Phase 0 and Phase 1 transcripts are warm:
  `continue_from` the last reference for follow-ups; start a fresh one for a new
  phase.
- **`main` lags `upstream/main`** (`14562ee6` vs `72463208`). The register is
  measured against `upstream/main`; the next `scripts/fork/sync-upstream.sh`
  fast-forwards `main`.
- **Where the page may live.** Not `app/src/fork/**`: `mod console;` and
  `mod game;` are private, `CollectionView`'s fields are private, and no
  `impl CollectionView` exists outside `app/src/collection/**`. The page is
  fork-owned files *inside* `collection/console/watch_page*`.
- **Never branch on the space in upstream layout bodies** (`game/hero.rs`,
  `game/tabs.rs`, `game/sheet.rs`, `game/showcase.rs`, `layouts/**` may still
  *word* things per space under F-016/F-021). The guards enforce the dispatch
  part.
- **`game_rows` + `game_press_row` are not enough** for this page: four upstream
  layouts map `Outcome::Changed` to Activate (a season ◀ ▶ would play an episode),
  Sheet filters rows by a six-id allow-list, Hero has no row surface, and
  `Row` cannot carry a still image (`console/list.rs` takes a `PathBuf`).
- **i18n**: every string lives in `crates/i18n/locales/{de,es,fr,pt-BR}`; the
  coverage test fails on a missing or unused key. The Watch labels already exist
  ("Watched", "Last watched", "{time} watched", "Not watched yet") — reuse them.

## Recorded nits (not blocking)

1. The film Store line has no unit test; its input (`fork::watch::sources::origins`)
   is tested in `crates/watch/sources.rs` equivalent (`fork/watch/sources.rs`).
2. Rail presses during the "titles missing" window are untested.
3. `app/src/fork/seams.rs`'s spotlight row cites F-012 and F-010 but its `why`
   does not mention F-021.

## Sync log — the parity measurement

`docs/WATCH-PAGE.md` §10 re-opens the "host Watch inside upstream's layouts"
question when the detail-page files go two syncs without a change. Count them
after each `scripts/fork/sync-upstream.sh` and record it here:

| Date | Previous upstream | Upstream | `console/game/*.rs` changed | `console/layouts/**` changed | Verdict |
|---|---|---|---|---|---|
| 2026-11-21 | `14562ee6` | `72463208` | 4 (hero, mod, sheet, showcase; `tabs.rs` untouched) | 13 | stay out |

The count is `git diff <previous upstream> <new upstream> --numstat -- <path>`,
counted by file; record both commits so the numbers can be reproduced.
An unchanged sync is the only evidence that re-grafting would stop being a
treadmill; the decision is not re-argued without two of them in a row.

## Open owner decisions

- Which styles Watch ships besides **Film** (Phase 3), and the parity check's
  form.
- Whether the engine absorbs `watch_playback.rs`'s view state (it keeps data
  state either way).

## Hold — 2026-11-22, waiting on upstream #277

Upstream is reworking how pages are templated (`RafaelGoulartB/Heronite#277`).
Until that lands the fork adds no page features: wiring their Classic/Tabs/Sheet/
Showcase as Watch options would be built on the composition model that issue is
replacing. Nothing is half-built — the last code commit is `5ff3161b`.

Shipped and settled:

- **Stage** (title, marks and list on the left, details panel beside them) and
  **Stage Wide** (title band across the top, two full-width cards), chosen per
  space in Settings → Watch and stored in `console-watch.json`; `film`/`cinema`
  still parse, so an older file keeps its choice.
- The film page is fork-owned (`app/src/collection/console/watch_page/`); the
  upstream edits are the registered seam set in `app/src/fork/seams.rs`, with
  guards that fail by name when a token moves.

Open, to be re-decided against #277 rather than built now:

1. **Upstream layouts as Watch options.** Needs the five shell answers in
   `console/game/mod.rs` to branch on the *layout* rather than the space (7
   sites), `episode:`/`season` presses in `game_pad_classic` (only `stream:` is
   handled), and one `details_card` helper — it is duplicated in the two Stage
   arms, so a change today has to be made twice.
2. **`game_rows`'s Watch branch is unreachable** until (1): every layout returns
   early for Watch. `ab8c72ea` claimed it worked and was reverted for it; feed it
   with (1), not before.
3. **Hero has no list at all** — excluded from Watch until someone gives it one.
4. **Rails draw 2 cards per row**: `columns_in` rounds down and throws away the
   panel's third. Rounding up is a two-line change, but it invalidates
   `a_rails_row_never_runs_out_of_the_panel`, which asserts the tiles never
   shrink — rewrite that test with the change, not after.
5. The setting row still reads "Film page".

## Restart checklist

1. `git log --oneline -6` — expect `5ff3161b` on top of a revert. That revert is
   deliberate; do not re-apply `ab8c72ea`.
2. Read `docs/WATCH-PAGE.md` (the decision), this file (the state), `docs/FORK.md`
   (the seams and the rules).
3. `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
   && cargo test --workspace && cargo test -p heronite-i18n --test coverage`.
4. Look at upstream first: if #277 has landed, read how pages compose now before
   touching 1–5 above. If it has not, the only safe work here is (4) and (5).

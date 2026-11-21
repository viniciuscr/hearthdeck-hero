# An upstream extension point for the console detail page

A proposal for Heronite (upstream), prepared by the Hearthdeck Hero fork under the
fork's upstream-first rule (`docs/FORK.md`). Nothing here is a request for the
fork's own features: it is a generic hook, useful to any client that has more
than one kind of title.

## The problem, in upstream's terms

Upstream's console detail page picks one of five styles with `theme.game_page`
and draws it from `console/game/{mod,hero,tabs,sheet,showcase}.rs`. Everything
those files draw is a *game*: install state, status, playtime, How Long To Beat,
screenshots, journal, achievements, and a `related` strip from the game library.

A client whose library also holds films or series has no way to contribute to
that page. The page's inputs are `CollectionGame` rows and a `FocusVm`
(`pub(super)`), its geometry is precomputed in a closed `Zone` enum, and its
sections are hard-coded. So the only options are (a) edit the five drawing
routines, or (b) ship a page of one's own and never inherit their improvements.
The fork measured (a) at 11 → ≥14 modified upstream files, with the edits landing
in the files upstream rewrites most often.

## What is proposed

One trait, implemented by the app, optional at compile time:

```rust
/// A page contributed by a client that has kinds of title this app does not
/// model. Answered by the shell wherever it asks the game page a question.
pub trait DetailPage {
    /// The screen itself. The host draws its own background and bars.
    fn render(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<CollectionView>) -> AnyElement;

    /// The screen's pad handling. `false` lets the shell use the press.
    fn pad(&mut self, key: &SharedString, pad: Pad, window: &mut Window, cx: &mut Context<CollectionView>) -> bool;

    /// The hint bar for this screen.
    fn hints(&self, key: &SharedString) -> Vec<Hint>;

    /// The scrim drawn behind it, and whether the tab bar stays.
    fn scrim(&self, spec: &TabSpec) -> Scrim { Scrim::Classic }
    fn keeps_tabs(&self) -> bool { false }

    /// Whether the wheel scrolls the page's text (so the shell routes it here).
    fn scrolls_text(&self) -> bool { true }

    /// What floats over the page, if anything (the Sheet's panel).
    fn layer(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<CollectionView>) -> Option<AnyElement> { None }
}
```

And one question at the seven places the shell already asks the game page:

```rust
// console/mod.rs, beside the existing game-page calls
if let Some(page) = self.detail_page(key) { return page.render(key, window, cx); }
```

with `detail_page` returning `Option<&mut dyn DetailPage>` from a client-provided
registry — empty by default, so upstream ships unchanged and pays nothing.

## Why this shape

- **It matches what the shell already does.** The seven questions above are
  literally the seven the fork answers today by branching on its space
  (`render_console_game`, `render_console_game_layer`, `game_pad`, `game_hints`,
  `game_scrim`, `game_page_scrolls_text`, `game_page_keeps_tabs`). The trait is
  the same contract, named once instead of written seven times per client.
- **A section hook would be better long-term, but needs more design.** Letting a
  client contribute *sections* into the existing styles requires theming heights
  and focus (`Shape`/`Zone`/`Geo`) to accept foreign sections. That is a real
  refactor; the page hook above is a day's work and unblocks clients now.
- **Zero cost when unused.** One `Option` behind a registry; no allocations on
  the path the app takes today.
- **It keeps upstream's ownership of its own styles.** No client gains the right
  to edit them; a client's page is its own, and upstream's improvements to its
  five styles keep reaching its users.

## What the fork would do if this lands

Delete its seven branches in favour of one implementation of the trait, keeping
`app/src/fork/seams.rs`'s debt list shrinking to the input-stack hooks. The
fork's page becomes a `DetailPage`, and `docs/WATCH-PAGE.md` §10 is superseded by
upstream's contract rather than the fork's.

## How to send it

Per `docs/FORK.md`: a branch cut from `upstream/main`, never from `fork/main`,
with no fork commits and no fork branding — the trait, the registry, the seven
call sites, a test that the registry is empty by default, and a paragraph in the
changelog. The fork's own page is *not* part of the proposal.

**This needs the maintainer's own GitHub access to open.** The fork's rule is
that fork commits are never pushed upstream, so the branch and the pull request
are a person's action, not an agent's.

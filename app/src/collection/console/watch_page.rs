//! The film page in console mode: our own page for Watch titles.
//!
//! Fork-owned (F-012). It is composed from the house pieces — `console::kit`,
//! the console list, the Collection cover loader, the focus card surface and
//! the games' own facts table — so it looks and behaves like the rest of the
//! console. The games' page in `console/game/` stays upstream's: it gains one
//! dispatch line and nothing else, so its improvements keep coming.
//!
//! What it shows is what a film is, never what a game is: no installed state,
//! playtime, status, folders, screenshots or HowLongToBeat. The description
//! and the details come from the metadata sources (`heronite_watch::metadata`:
//! Cinemeta today), while Play and its rows are the provider's streams.
//!
//! A series gets a picker in the same list: a season row moved with ◀ ▶, one row
//! per episode of that season, and the picked episode's streams under them. A
//! on an episode plays its best stream at once; the card previews what the
//! picker is on.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::text::DateLocale;
use heronite_data::CollectionGame;
use heronite_i18n::tr;
use heronite_images::{ImageRef, Variant};
use heronite_watch::metadata::{Episode, TitleDetails};

use super::super::watch_playback::Selection;
use super::super::CollectionView;
use super::game::facts_table;
use super::hero::{card_surface, fitted_title};
use crate::console::kit::{self, u};
use crate::console::list::{Outcome, Row, Value};
use crate::console::motion;
use crate::console::Pad;
use crate::icons::Icon;
use crate::theme;

/// One label / value line of the page's card.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DetailLine {
    label: &'static str,
    value: String,
}

/// The line under a film's title: who owns it, then what the source knows
/// (`Stremio · 1999 · 136 min · IMDb 8.7`).
fn facts_line(provider: &str, details: Option<&TitleDetails>, library_year: Option<&str>) -> String {
    let mut facts = vec![provider.to_owned()];
    let year = details.and_then(|details| details.year.clone()).or_else(|| library_year.map(str::to_owned));
    facts.extend(year);
    if let Some(details) = details {
        facts.extend(details.runtime.clone());
        facts.extend(details.rating.as_ref().map(|rating| tr!("IMDb {rating}", rating = rating).to_string()));
    }
    facts.retain(|fact| !fact.trim().is_empty());
    facts.join(" \u{b7} ")
}

/// "23m watched · Last watched Nov 12, 2026", `None` when never watched.
fn watched_line(playtime_minutes: u64, last_played: Option<&str>, locale: DateLocale) -> Option<String> {
    let mut parts = Vec::new();
    if playtime_minutes > 0 {
        parts.push(tr!("{time} watched", time = heronite_data::collection::format_playtime_minutes(playtime_minutes)).to_string());
    }
    let watched = last_played.map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty());
    parts.extend(watched.map(|at| tr!("Last watched {at}", at = at).to_string()));
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// The names as one line, `None` when the source listed none.
fn names(values: &[String]) -> Option<String> {
    (!values.is_empty()).then(|| values.join(", "))
}

/// The card's lines: the source's fields, then the library's genres when the
/// source had none (or could not be asked).
fn detail_lines(details: Option<&TitleDetails>, library_genres: &[String], locale: DateLocale) -> Vec<DetailLine> {
    let mut lines: Vec<DetailLine> = Vec::new();
    let mut push = |label: &'static str, value: Option<String>| {
        if let Some(value) = value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()) {
            lines.push(DetailLine { label, value });
        }
    };
    if let Some(details) = details {
        push(tr!("Directed by"), names(&details.directors));
        push(tr!("Written by"), names(&details.writers));
        push(tr!("Starring"), names(&details.cast));
        push(
            tr!("Released"),
            details.released.as_deref().map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty()),
        );
        push(tr!("Runtime"), details.runtime.clone());
        push(tr!("Rating"), details.rating.clone());
        push(tr!("Genres"), names(&details.genres));
        push(tr!("Country"), names(&details.countries));
        push(tr!("Awards"), details.awards.clone());
        push(tr!("Status"), details.series_status.clone());
        push(tr!("Seasons"), (details.season_count() > 0).then(|| details.season_count().to_string()));
        push(tr!("Episodes"), (details.episode_count() > 0).then(|| details.episode_count().to_string()));
    }
    if details.is_none_or(|details| details.genres.is_empty()) {
        push(tr!("Genres"), names(library_genres));
    }
    lines
}

/// "Winter Is Coming", or "Episode 3" when the source has no name.
fn episode_name(episode: &Episode) -> String {
    match episode.name.trim() {
        "" => tr!("Episode {n}", n = episode.number).to_string(),
        name => name.to_owned(),
    }
}

/// "1 · Winter Is Coming": the picker row's label.
fn episode_label(episode: &Episode) -> String {
    format!("{} · {}", episode.number, episode_name(episode))
}

/// "S1E2 · 18 Apr 2011": the picker row's second line.
fn episode_desc(episode: &Episode, locale: DateLocale) -> String {
    let mut parts = vec![episode.code()];
    parts
        .extend(episode.released.as_deref().map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty()));
    parts.join(" \u{b7} ")
}

/// One row of a season's episodes ("episode:{index}" in the list); the episode
/// the stored progress points at wears a tick.
fn episode_row(episode: &Episode, index: usize, watched: bool, locale: DateLocale) -> Row {
    let row = Row::action(SharedString::from(format!("episode:{index}")), episode_label(episode)).desc(episode_desc(episode, locale));
    row.icon(if watched { Icon::Check } else { Icon::PlayArrow })
}

/// The episode the picker shows: the chosen one of the chosen season.
fn picked_episode(details: &TitleDetails, selection: Selection) -> Option<&Episode> {
    let season = details.seasons().get(selection.season).copied()?;
    details.episodes_of(season).nth(selection.episode)
}

/// The picked episode above the title's own details: still, code, name, date,
/// rating and what it is about.
fn episode_card(episode: &Episode, still: Option<ImageSource>, locale: DateLocale) -> Div {
    let mut line = vec![episode.code()];
    line.extend(episode.released.as_deref().map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty()));
    line.extend(episode.rating.as_ref().map(|rating| tr!("IMDb {rating}", rating = rating).to_string()));
    div()
        .flex()
        .flex_col()
        .gap(u(10.))
        .child(
            div()
                .flex_none()
                .w(u(420.))
                .h(u(236.))
                .rounded(u(14.))
                .overflow_hidden()
                .bg(kit::ink(0.08))
                .children(still.map(|still| img(still).size_full().object_fit(ObjectFit::Cover))),
        )
        .child(div().text_size(u(20.)).font_weight(theme::W_BOLD).text_color(theme::text_primary()).child(format!(
            "{} \u{b7} {}",
            episode.code(),
            episode_name(episode)
        )))
        .children((!line.is_empty()).then(|| div().text_size(u(14.)).text_color(theme::text_muted()).child(line.join(" \u{b7} "))))
        .children(
            episode.overview.clone().filter(|overview| !overview.trim().is_empty()).map(|overview| {
                div().max_w(u(640.)).text_size(u(15.)).line_height(u(22.)).text_color(theme::text_primary()).child(overview)
            }),
        )
}

impl CollectionView {
    /// The console page of a film or a series.
    pub(super) fn render_watch_game_page(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(g) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else {
            return kit::empty(Icon::Warning, tr!("This title is gone"), tr!("Press B to go back.")).into_any_element();
        };
        self.console_focus_game(Some(key.clone()), cx);
        self.watch_ensure_details(&g, cx);

        // Everything the page draws is taken before the view is borrowed mutably
        // (the list, the cover and the scroll driver all need `&mut self`).
        let details = self.watch_details(key);
        let failed = self.watch_details_failed(key);
        // A series opens on the episode the stored progress points at.
        let selection =
            details.as_deref().filter(|details| !details.episodes.is_empty()).map(|details| self.watch_ensure_selection(key, details));
        let picked = match (details.as_deref(), selection) {
            (Some(details), Some(selection)) => picked_episode(details, selection).cloned(),
            _ => None,
        };
        let locale = self.lib.as_ref().map(|lib| lib.date_locale).unwrap_or_default();
        let facts = facts_line(g.source_label, details.as_deref(), crate::fork::watch::sources::year(&g));
        let watched = watched_line(g.playtime_minutes, g.last_played.as_deref(), locale);
        let lines = detail_lines(details.as_deref(), &g.details.genres, locale);
        let overview = details
            .as_deref()
            .and_then(|details| details.description_or(Some(g.details.description.as_str())))
            .map(str::to_owned)
            .filter(|text| !text.trim().is_empty());

        let rows = self.watch_rows(&g, details.as_deref(), locale, cx);
        let list = self.console.game.list.render("console-game-actions", &rows, true);
        let s = kit::scale();
        let still = picked.as_ref().and_then(|episode| episode.thumbnail.as_deref()).and_then(|url| self.episode_still(url, 420., 236., s));
        let (cw, ch) = (136., 136. * self.cover_aspect);
        let cover = self.cover(key, cw * s, ch * s, window.scale_factor(), 300);
        let title = fitted_title(52., g.title.chars().count());
        let columns = if card_is_wide(window, s) { 2 } else { 1 };

        let header = div()
            .flex()
            .items_end()
            .gap(u(28.))
            .px(u(46.))
            .pt(u(64.))
            .pb(u(24.))
            .child(
                div()
                    .flex_none()
                    .w(u(cw))
                    .h(u(ch))
                    .rounded(u(crate::console::look::tile_radius() + 2.))
                    .overflow_hidden()
                    .bg(kit::ink(0.08))
                    .border_1()
                    .border_color(kit::ink(0.12))
                    .shadow(kit::shadow(0.6, 12., 34.))
                    .children(cover.map(|cover| img(cover).size_full().object_fit(ObjectFit::Cover))),
            )
            .child(motion::enter(
                SharedString::from(format!("watch-head-{key}")),
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .gap(u(8.))
                    .min_w_0()
                    .max_w(u(900.))
                    .pb(u(4.))
                    .child(kit::title(g.title.clone(), title).line_clamp(2))
                    .child(div().text_size(u(16.)).text_color(theme::text_muted()).child(facts))
                    .children(watched.map(|line| div().text_size(u(14.)).text_color(theme::text_dim()).child(line))),
                320,
                0.,
                10.,
            ));

        let mut about = div()
            .id("console-watch-about")
            .flex()
            .flex_col()
            .gap(u(20.))
            .overflow_y_scroll()
            .track_scroll(self.console.game.about.handle());
        if let Some(episode) = picked.as_ref() {
            about = about.child(episode_card(episode, still, locale));
        }
        if let Some(overview) = overview {
            about = about
                .child(kit::body_text(overview).max_w(u(980.)).text_size(u(16.)).line_height(u(25.)).text_color(theme::text_primary()));
        }
        if !lines.is_empty() {
            let table: Vec<(SharedString, String)> =
                lines.iter().map(|line| (SharedString::from(line.label), line.value.clone())).collect();
            about = about.child(div().h(px(1.)).flex_none().bg(kit::ink(0.08))).child(facts_table(&table, columns));
        }
        if failed {
            about = about.child(div().text_size(u(14.)).text_color(theme::text_dim()).child(tr!("Details unavailable right now.")));
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(header)
            .child(self.console.game.about.driver())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap(u(40.))
                    .px(u(46.))
                    .pb(u(20.))
                    .child(div().flex_none().w(u(560.)).flex().flex_col().child(list.flex_1().min_h_0()))
                    .child(card_surface().flex_1().min_w_0().p(u(26.)).rounded(u(20.)).flex().flex_col().child(about.flex_1().min_h_0())),
            )
            .into_any_element()
    }
}

impl CollectionView {
    /// The page's rows: Play, then for a series the season row, that season's
    /// episodes and the picked episode's streams; a film lists its own streams.
    fn watch_rows(
        &mut self,
        game: &CollectionGame,
        details: Option<&TitleDetails>,
        locale: DateLocale,
        cx: &mut Context<Self>,
    ) -> Vec<Row> {
        let (label, icon) = self.primary_state(game);
        let mut rows = vec![Row::action("primary", label).icon(icon)];
        let Some(details) = details.filter(|details| !details.episodes.is_empty()) else {
            self.watch_ensure_streams(game, None, cx);
            rows.extend(self.watch_stream_rows(game, None));
            return rows;
        };
        let selection = self.watch_ensure_selection(&game.key, details);
        let seasons = details.seasons();
        let season = seasons.get(selection.season).copied().unwrap_or(1);
        let options: Vec<SharedString> = seasons.iter().map(|number| SharedString::from(tr!("Season {n}", n = number))).collect();
        rows.push(Row::choice("season", tr!("Season"), options, selection.season));
        let episodes: Vec<&Episode> = details.episodes_of(season).collect();
        let watched = self.watch_resume().map(str::to_owned);
        for (index, episode) in episodes.iter().enumerate() {
            rows.push(episode_row(episode, index, watched.as_deref() == Some(episode.id.as_str()), locale));
        }
        let video = episodes.get(selection.episode).map(|episode| episode.id.clone());
        self.watch_ensure_streams(game, video.as_deref(), cx);
        rows.extend(self.watch_stream_rows(game, video.as_deref()));
        rows
    }

    /// A on a row of the page.
    fn watch_press_row(&mut self, game: &CollectionGame, id: &str, cx: &mut Context<Self>) {
        if id == "primary" {
            self.watch_primary(game, cx);
            return;
        }
        if let Some(index) = id.strip_prefix("stream:").and_then(|index| index.parse::<usize>().ok()) {
            self.watch_play_stream(game, index, cx);
        } else if let Some(index) = id.strip_prefix("episode:").and_then(|index| index.parse::<usize>().ok()) {
            self.watch_press_episode(game, index, cx);
        }
    }

    /// A on an episode: it becomes the pick and its best stream starts at once.
    fn watch_press_episode(&mut self, game: &CollectionGame, index: usize, cx: &mut Context<Self>) {
        let Some(details) = self.watch_details(&game.key) else { return };
        let selection = self.watch_ensure_selection(&game.key, &details);
        let picked = Selection { season: selection.season, episode: index };
        let Some(episode) = picked_episode(&details, picked) else { return };
        let video = episode.id.clone();
        self.watch_pick_episode(&game.key, picked);
        self.watch_play_episode(game, &video, cx);
    }

    /// A 16:9 still from its URL, once loaded (the loader caches it by size).
    fn episode_still(&mut self, url: &str, w: f32, h: f32, scale: f32) -> Option<ImageSource> {
        let (pw, ph) = ((w * scale).round() as u32, (h * scale).round() as u32);
        let id: SharedString = format!("still:{url}:{pw}x{ph}").into();
        let chain = ImageRef::Url(url.to_owned());
        self.images.get_or_request(&id, || vec![chain], pw, ph, Variant::Cover, 200).map(ImageSource::Render)
    }

    /// The page's presses: the list navigates, ◀ ▶ move the season, A plays the
    /// episode or the stream, X plays or stops, the wheel scrolls the card.
    pub(super) fn watch_page_pad(&mut self, key: &SharedString, pad: Pad, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(game) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else { return false };
        let details = self.watch_details(key);
        let locale = self.lib.as_ref().map(|lib| lib.date_locale).unwrap_or_default();
        let rows = self.watch_rows(&game, details.as_deref(), locale, cx);
        match self.console.game.list.on_pad(pad, &rows) {
            Outcome::Activate(id) => {
                self.watch_press_row(&game, &id, cx);
                cx.notify();
                true
            }
            Outcome::Changed(id, Value::Choice { index, .. }) if id.as_ref() == "season" => {
                if let Some(details) = self.watch_details(key) {
                    self.watch_pick_season(&game.key, &details, index);
                    self.watch_forget_streams();
                    cx.notify();
                }
                true
            }
            Outcome::Moved | Outcome::Edge(_) | Outcome::Changed(..) => {
                cx.notify();
                true
            }
            Outcome::Ignored => match pad {
                Pad::X => {
                    self.primary_action(key, cx);
                    true
                }
                Pad::ScrollUp | Pad::ScrollDown => {
                    let step = f32::from(u(120.));
                    self.console.game.about.by(if pad == Pad::ScrollDown { step } else { -step });
                    true
                }
                _ => false,
            },
        }
    }
}

/// Whether the card has room for two columns of facts (design px, as the
/// games' page measures it).
fn card_is_wide(window: &Window, scale: f32) -> bool {
    let viewport = f32::from(crate::ui::content_size(window).width) / scale;
    viewport - 2. * 46. - 560. - 40. > 760.
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings gpui's `test` macro into scope.
    use core::prelude::v1::test;

    fn matrix() -> TitleDetails {
        TitleDetails {
            name: "The Matrix".into(),
            year: Some("1999".into()),
            runtime: Some("136 min".into()),
            rating: Some("8.7".into()),
            released: Some("1999-03-31".into()),
            genres: vec!["Action".into(), "Sci-Fi".into()],
            directors: vec!["Lana Wachowski".into(), "Lilly Wachowski".into()],
            writers: vec!["Lilly Wachowski".into()],
            cast: vec!["Keanu Reeves".into(), "Carrie-Anne Moss".into()],
            countries: vec!["United States".into()],
            awards: Some("Won 4 Oscars.".into()),
            ..TitleDetails::default()
        }
    }

    #[test]
    fn the_header_names_the_provider_and_what_the_source_knows() {
        assert_eq!(facts_line("Stremio", Some(&matrix()), None), "Stremio · 1999 · 136 min · IMDb 8.7");
        assert_eq!(facts_line("Stremio", None, Some("1999")), "Stremio · 1999");
        assert_eq!(facts_line("Stremio", None, None), "Stremio");
    }

    #[test]
    fn the_card_lists_the_films_people_and_no_game_row() {
        let lines = detail_lines(Some(&matrix()), &[], DateLocale::En);
        let labels: Vec<&str> = lines.iter().map(|line| line.label).collect();
        assert_eq!(labels, ["Directed by", "Written by", "Starring", "Released", "Runtime", "Rating", "Genres", "Country", "Awards"]);
        assert_eq!(lines[0].value, "Lana Wachowski, Lilly Wachowski");
        assert_eq!(lines[3].value, "3/31/1999", "the released date follows the library's locale");
        for game in ["Playtime", "Last Played", "Installed", "Folders", "Steam tags", "Series"] {
            assert!(!labels.contains(&game), "{game} is not a film fact");
        }
    }

    fn episode(season: u32, number: u32, name: &str) -> Episode {
        Episode {
            id: format!("tt0944947:{season}:{number}"),
            season,
            number,
            name: name.into(),
            thumbnail: Some(format!("https://episodes.metahub.space/tt0944947/{season}/{number}/w780.jpg")),
            released: Some(format!("2011-0{season}-1{number}")),
            ..Episode::default()
        }
    }

    fn series() -> TitleDetails {
        TitleDetails {
            kind: "series".into(),
            series_status: Some("Ended".into()),
            episodes: vec![episode(1, 1, "Winter Is Coming"), episode(1, 2, "The Kingsroad"), episode(2, 1, "The North Remembers")],
            ..matrix()
        }
    }

    #[test]
    fn a_series_adds_its_status_and_counts() {
        let details = series();
        let lines = detail_lines(Some(&details), &[], DateLocale::En);
        let value = |label: &str| lines.iter().find(|line| line.label == label).map(|line| line.value.as_str());
        assert_eq!(value("Status"), Some("Ended"));
        assert_eq!(value("Seasons"), Some("2"));
        assert_eq!(value("Episodes"), Some("3"));
    }

    #[test]
    fn the_picker_names_each_episode_of_the_chosen_season() {
        let details = series();
        let selection = Selection { season: 0, episode: 1 };
        let picked = picked_episode(&details, selection).expect("the season has two episodes");
        assert_eq!(picked.code(), "S1E2");
        assert_eq!(episode_label(picked), "2 \u{b7} The Kingsroad");
        assert_eq!(episode_desc(picked, DateLocale::En), "S1E2 \u{b7} 1/12/2011");
        // The row's id says which episode of the season the list means.
        assert_eq!(episode_row(picked, 1, false, DateLocale::En).id.as_ref(), "episode:1");
        assert!(episode_row(picked, 1, true, DateLocale::En).icon.is_some(), "a watched episode wears a tick");

        let other = Selection { season: 1, episode: 0 };
        assert_eq!(picked_episode(&details, other).map(|episode| episode.name.as_str()), Some("The North Remembers"));
        assert_eq!(picked_episode(&details, Selection { season: 9, episode: 0 }), None, "no such season");
    }

    #[test]
    fn an_unnamed_episode_is_its_number() {
        let unnamed = Episode { season: 1, number: 7, ..Episode::default() };
        assert_eq!(episode_name(&unnamed), "Episode 7");
        assert_eq!(episode_label(&unnamed), "7 \u{b7} Episode 7");
    }

    #[test]
    fn without_a_source_the_library_genres_stand_in() {
        let lines = detail_lines(None, &["Drama".into()], DateLocale::En);
        assert_eq!(lines, [DetailLine { label: "Genres", value: "Drama".into() }]);
        assert!(detail_lines(None, &[], DateLocale::En).is_empty());
    }

    #[test]
    fn a_watched_film_says_how_much_and_when() {
        assert_eq!(watched_line(23, None, DateLocale::En).as_deref(), Some("23m watched"));
        assert_eq!(watched_line(0, None, DateLocale::En), None);
        let line = watched_line(0, Some("2026-11-12"), DateLocale::En).expect("a date is a fact");
        assert!(line.starts_with("Last watched "), "{line}");
    }
}

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

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_data::text::DateLocale;
use heronite_i18n::tr;
use heronite_watch::metadata::TitleDetails;

use super::super::CollectionView;
use super::game::facts_table;
use super::hero::{card_surface, fitted_title};
use crate::console::kit::{self, u};
use crate::console::motion;
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
        push(tr!("Seasons"), details.seasons.map(|count| count.to_string()));
        push(tr!("Episodes"), details.episodes.map(|count| count.to_string()));
    }
    if details.is_none_or(|details| details.genres.is_empty()) {
        push(tr!("Genres"), names(library_genres));
    }
    lines
}

impl CollectionView {
    /// The console page of a film or a series.
    pub(super) fn render_watch_game_page(&mut self, key: &SharedString, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(g) = self.lib.as_ref().and_then(|lib| lib.get(key)).cloned() else {
            return kit::empty(Icon::Warning, tr!("This title is gone"), tr!("Press B to go back.")).into_any_element();
        };
        self.console_focus_game(Some(key.clone()), cx);
        self.watch_ensure_streams(&g, cx);
        self.watch_ensure_details(&g, cx);

        // Everything the page draws is taken before the view is borrowed mutably
        // (the list, the cover and the scroll driver all need `&mut self`).
        let details = self.watch_details(key);
        let failed = self.watch_details_failed(key);
        let locale = self.lib.as_ref().map(|lib| lib.date_locale).unwrap_or_default();
        let facts = facts_line(g.source_label, details.as_deref(), crate::fork::watch::sources::year(&g));
        let watched = watched_line(g.playtime_minutes, g.last_played.as_deref(), locale);
        let lines = detail_lines(details.as_deref(), &g.details.genres, locale);
        let overview = details
            .as_deref()
            .and_then(|details| details.description_or(Some(g.details.description.as_str())))
            .map(str::to_owned)
            .filter(|text| !text.trim().is_empty());

        let rows = self.game_rows(&g, cx);
        let list = self.console.game.list.render("console-game-actions", &rows, true);
        let s = kit::scale();
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

    #[test]
    fn a_series_adds_its_status_and_counts() {
        let details =
            TitleDetails { kind: "series".into(), series_status: Some("Ended".into()), seasons: Some(8), episodes: Some(73), ..matrix() };
        let lines = detail_lines(Some(&details), &[], DateLocale::En);
        let value = |label: &str| lines.iter().find(|line| line.label == label).map(|line| line.value.as_str());
        assert_eq!(value("Status"), Some("Ended"));
        assert_eq!(value("Seasons"), Some("8"));
        assert_eq!(value("Episodes"), Some("73"));
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

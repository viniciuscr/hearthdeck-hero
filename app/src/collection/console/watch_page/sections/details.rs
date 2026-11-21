//! The film page's facts: the line under the title, its standing, how long and
//! when, and the card's label / value lines.

use gpui_kit::*;
use heronite_data::text::DateLocale;
use heronite_i18n::{tr, trn};
use heronite_watch::metadata::{Fact, TitleDetails};

use super::super::DetailLine;

/// The line under a film's title: where it comes from, then what the source
/// knows (`Cinemeta · Stremio · 1999 · 136 min · IMDb 8.7`). The origins are
/// `sources::origins_with`, so the stream provider appears only once a stream is
/// known to exist.
pub(in crate::collection::console::watch_page) fn facts_line(
    origins: &[&str],
    details: Option<&TitleDetails>,
    library_year: Option<&str>,
) -> String {
    let mut facts: Vec<String> = origins.iter().map(|origin| (*origin).to_owned()).collect();
    let year = details.and_then(|details| details.year.clone()).or_else(|| library_year.map(str::to_owned));
    facts.extend(year);
    if let Some(details) = details {
        facts.extend(details.runtime.clone());
        facts.extend(details.rating.as_ref().map(|rating| tr!("IMDb {rating}", rating = rating).to_string()));
    }
    facts.retain(|fact| !fact.trim().is_empty());
    facts.join(" \u{b7} ")
}

/// The title's standing in one line, before how long and when: "Watched", or
/// "In progress · 40 %" for a film left part-way, or "3 of 8 episodes watched"
/// for a series. `None` when nothing of it was played — or, for a series, when
/// the page does not know the episode list and there is nothing to count against.
pub(in crate::collection::console::watch_page) fn title_status(
    progress: Option<&heronite_watch::progress::MovieProgress>,
    kind: &str,
    details: Option<&TitleDetails>,
) -> Option<String> {
    use heronite_watch::progress::WatchedState;
    let progress = progress?;
    if kind == "series" {
        let standing = progress.series_standing();
        if standing.watched == 0 {
            return None;
        }
        let total = details.filter(|details| !details.episodes.is_empty()).map(|details| details.episode_count());
        return Some(match total {
            Some(1) => tr!("{watched} of 1 episode watched", watched = standing.watched).to_string(),
            Some(total) => tr!("{watched} of {total} episodes watched", watched = standing.watched, total = total).to_string(),
            None => trn!(standing.watched, "{n} episode watched", "{n} episodes watched"),
        });
    }
    match progress.state() {
        WatchedState::Watched => Some(tr!("Watched").to_string()),
        WatchedState::InProgress { .. } => progress.state().percent().map(|percent| tr!("In progress · {percent}%", percent = percent)),
        WatchedState::Unwatched => None,
    }
}

/// "Watched · 23m watched · Last watched Nov 12, 2026", `None` when nothing says
/// anything. A film says **Watched** only once its own record reached the
/// threshold; the position alone (opened and abandoned) is not watching it.
pub(in crate::collection::console::watch_page) fn watched_line(
    playtime_minutes: u64,
    last_played: Option<&str>,
    status: Option<String>,
    locale: DateLocale,
) -> Option<String> {
    let mut parts = Vec::new();
    parts.extend(status);
    if playtime_minutes > 0 {
        parts.push(tr!("{time} watched", time = heronite_data::collection::format_playtime_minutes(playtime_minutes)).to_string());
    }
    let watched = last_played.map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty());
    parts.extend(watched.map(|at| tr!("Last watched {at}", at = at).to_string()));
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// The names as one line, `None` when the source listed none.
pub(in crate::collection::console::watch_page) fn names(values: &[String]) -> Option<String> {
    (!values.is_empty()).then(|| values.join(", "))
}

/// The card's lines: the source's fields, then the library's genres when the
/// source had none (or could not be asked).
pub(in crate::collection::console::watch_page) fn detail_lines(
    details: Option<&TitleDetails>,
    library_genres: &[String],
    locale: DateLocale,
) -> Vec<DetailLine> {
    let mut lines: Vec<DetailLine> = Vec::new();
    let mut push = |label: SharedString, value: Option<String>| {
        if let Some(value) = value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty()) {
            lines.push(DetailLine { label, value });
        }
    };
    if let Some(details) = details {
        push(tr!("Directed by").into(), names(&details.directors));
        push(tr!("Written by").into(), names(&details.writers));
        push(tr!("Starring").into(), names(&details.cast));
        push(
            tr!("Released").into(),
            details.released.as_deref().map(|at| heronite_data::text::format_timestamp(at, locale)).filter(|at| !at.trim().is_empty()),
        );
        let series = details.kind == "series";
        // The sources give a show one episode's length, never the whole run: the
        // label says which it is, and the binge sums the episodes it lists.
        push(if series { tr!("Episode runtime") } else { tr!("Runtime") }.into(), details.runtime.clone());
        if series {
            let per_episode = details.runtime.as_deref().and_then(crate::fork::watch::sources::runtime_minutes_of);
            let binge = crate::fork::watch::sources::binge_minutes(per_episode, details.episode_count());
            push(tr!("Time to binge").into(), binge.map(heronite_data::collection::format_playtime_minutes));
        }
        push(tr!("Rating").into(), details.rating.clone());
        push(tr!("Genres").into(), names(&details.genres));
        push(tr!("Country").into(), names(&details.countries));
        push(tr!("Awards").into(), details.awards.clone());
        push(tr!("Status").into(), details.series_status.clone());
        push(tr!("Seasons").into(), (details.season_count() > 0).then(|| details.season_count().to_string()));
        push(tr!("Episodes").into(), (details.episode_count() > 0).then(|| details.episode_count().to_string()));
        // Whatever else the chosen sources know and we do not model: a tagline, a
        // budget, a network, the original language. A source that knows more adds
        // rows without this page changing.
        for fact in &details.extras {
            push(fact_label(fact), Some(fact.value.clone()));
        }
    }
    if details.is_none_or(|details| details.genres.is_empty()) {
        push(tr!("Genres").into(), names(library_genres));
    }
    lines
}

/// A fact's row name: the ids we know read in the person's language, anything
/// else as the source wrote it.
pub(in crate::collection::console::watch_page) fn fact_label(fact: &Fact) -> SharedString {
    match fact.id.as_str() {
        "tagline" => tr!("Tagline").into(),
        "budget" => tr!("Budget").into(),
        "network" => tr!("Network").into(),
        "certification" => tr!("Certification").into(),
        "original_language" => tr!("Original language").into(),
        _ if fact.label.trim().is_empty() => SharedString::from(fact.id.clone()),
        _ => SharedString::from(fact.label.clone()),
    }
}

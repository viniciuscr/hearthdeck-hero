//! The Watch space's facts in one place: how a time is worded, the line under a
//! title's name, and the labels its detail rows carry.
//!
//! Fork-owned, an addition rather than a deviation (`docs/FORK.md`). The console
//! layouts print these strings, and the desktop panel's rows carry the labels
//! `relabel` writes, so a new upstream surface asks here instead of growing its
//! own `match self.space` (`docs/WATCH-PAGE.md` §6, Phase 1). Games keep saying
//! played and Playtime; a film or a series says watched and Watched.

use heronite_data::organize::release_year;
use heronite_data::CollectionGame;
use heronite_i18n::tr;

use super::data::played_ago;
use super::vm::{DetailField, DetailRow};
use super::Space;

/// The time `g` was watched ("3h 54m"), `None` when it never was.
pub(super) fn watched_time(g: &CollectionGame) -> Option<String> {
    (g.playtime_minutes > 0).then(|| heronite_data::collection::format_playtime_minutes(g.playtime_minutes))
}

/// "{time} watched" in Watch, "{time} played" in Games.
pub(super) fn time_label(space: Space, time: &str) -> String {
    match space {
        Space::Games => tr!("{time} played", time = time),
        Space::Watch => tr!("{time} watched", time = time),
    }
}

/// The "when" half of a played line: "Last played 3 days ago" for a game,
/// "Last watched 3 days ago" for a film, and the never-started wording.
pub(super) fn when_label(space: Space, ago: Option<&str>, long: bool) -> String {
    let watch = space == Space::Watch;
    match ago {
        Some(ago) if long && watch => tr!("Last watched {at}", at = ago.to_lowercase()),
        Some(ago) if long => tr!("Last played {when}", when = ago.to_lowercase()),
        Some(ago) => ago.to_owned(),
        None if watch => tr!("Not watched yet").to_string(),
        None => tr!("Not played yet").to_string(),
    }
}

/// "3 days ago · 12h 40m"; `long` says "Last played 3 days ago · 12h 40m"
/// ("Last watched" and "watched" in the Watch space). "Not played yet" (or
/// "Not watched yet") for a title never started.
pub(super) fn played_line(space: Space, g: &CollectionGame, long: bool) -> String {
    let when = when_label(space, g.last_played.as_deref().and_then(played_ago).as_deref(), long);
    let time = watched_time(g).map(|time| time_label(space, &time));
    let parts: Vec<String> = Some(when).into_iter().chain(time).collect();
    parts.join(" \u{b7} ")
}

/// "Steam · Installed · 3h 54m played" for a game; "Cinemeta · 1999" (and
/// "Stremio" once a stream is known) for a film or a series — where it was
/// listed and what the source knows.
pub(super) fn facts_line(space: Space, g: &CollectionGame) -> String {
    let mut facts: Vec<String> = match space {
        Space::Games => vec![g.source_label.to_string()],
        // A catalogue stamps the provider that would resolve a stream, not one
        // that has it: the origin names the catalogue, and the stream source
        // joins it only when a probe found something.
        Space::Watch => crate::fork::watch::sources::origins(g).into_iter().map(str::to_owned).collect(),
    };
    match space {
        Space::Games => {
            facts.push(if g.installed { tr!("Installed").into() } else { tr!("Not installed").into() });
            facts.extend(watched_time(g).map(|time| time_label(Space::Games, &time)));
        }
        Space::Watch => {
            // The year, or the range a series prints ("2011–2019").
            let year = release_year(g)
                .map(|year| year.to_string())
                .or_else(|| g.details.release_date.as_deref().map(str::trim).filter(|date| !date.is_empty()).map(str::to_owned));
            facts.extend(year);
        }
    }
    facts.retain(|fact| !fact.trim().is_empty());
    facts.join(" \u{b7} ")
}

/// The labels a space gives its detail rows: "Last Played" and "Playtime" for a
/// game, "Last watched" and "Watched" for a film. A layout filters those rows by
/// field, never by the translated label.
pub(super) fn relabel(space: Space, rows: &mut [DetailRow]) {
    if space != Space::Watch {
        return;
    }
    for row in rows {
        match row.field {
            DetailField::LastPlayed => row.label = tr!("Last watched").into(),
            DetailField::Playtime => row.label = tr!("Watched").into(),
            _ => {}
        }
    }
}

/// "Steam · Installed" for a game. A film's Store row is its catalogues
/// (`store_line`): a catalogue lists a title, it does not install one.
pub(super) fn library_line(library: &str, installed: bool) -> String {
    let place = if installed { tr!("Installed") } else { tr!("Not installed") };
    format!("{library} \u{b7} {place}")
}

/// The Store row of the details panel. A film names the catalogues it was listed
/// in, as its hero line does — not the projection's fallback source label.
pub(super) fn store_line(space: Space, g: &CollectionGame, library: &str, installed: bool) -> String {
    match space {
        Space::Games => library_line(library, installed),
        Space::Watch => crate::fork::watch::sources::origins(g).join(" \u{b7} "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_film_says_watched_where_a_game_says_played() {
        assert_eq!(time_label(Space::Watch, "2h 10m"), "2h 10m watched");
        assert_eq!(time_label(Space::Games, "2h 10m"), "2h 10m played");
    }

    #[test]
    fn the_detail_rows_are_renamed_only_for_a_film() {
        let row = |field: DetailField, label: &str| DetailRow {
            field,
            label: label.into(),
            value: super::super::vm::DetailValue::Text("".into()),
            split: false,
        };
        let mut rows = vec![
            row(DetailField::LastPlayed, "Last Played"),
            row(DetailField::Playtime, "Playtime"),
            row(DetailField::Developer, "Developer"),
        ];
        relabel(Space::Watch, &mut rows);
        assert_eq!(rows[0].label.as_ref(), "Last watched");
        assert_eq!(rows[1].label.as_ref(), "Watched");
        assert_eq!(rows[2].label.as_ref(), "Developer", "a row that is not about time keeps its label");

        let mut games = vec![row(DetailField::Playtime, "Playtime")];
        relabel(Space::Games, &mut games);
        assert_eq!(games[0].label.as_ref(), "Playtime");
    }

    #[test]
    fn a_film_says_last_watched_where_a_game_says_last_played() {
        assert_eq!(when_label(Space::Watch, Some("3 days ago"), true), "Last watched 3 days ago");
        assert_eq!(when_label(Space::Games, Some("3 days ago"), true), "Last played 3 days ago");
        assert_eq!(when_label(Space::Watch, None, true), "Not watched yet");
        assert_eq!(when_label(Space::Games, None, true), "Not played yet");
        assert_eq!(when_label(Space::Games, Some("3 days ago"), false), "3 days ago", "the short form is the same in both spaces");
    }

    #[test]
    fn a_game_names_where_it_comes_from_and_whether_it_is_installed() {
        assert_eq!(library_line("Steam", true), "Steam \u{b7} Installed");
        assert_eq!(library_line("Steam", false), "Steam \u{b7} Not installed");
    }
}

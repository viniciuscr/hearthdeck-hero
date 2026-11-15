//! Watch items as a `Library` the existing Collection screens already read.
//!
//! The proxy writes a regenerable Heroic-format root under the app cache; the
//! facade loads it with `Library::load_with`. No Collection, console or
//! game-page code needs to know which provider the records came from.

use std::path::Path;

use anyhow::{Context, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_watch_core::collections::MARK_PREFIX;
use heronite_watch_core::types::MovieItem;
use serde_json::{json, Map, Value};

/// Where the proxy keeps its synthetic library.
pub const ROOT: &str = "watch-library";

/// The mark a catalog page leaves in its records, so a rails refresh keeps them.
pub const CATALOG_KEY: &str = "watchCatalog";

/// What a rails refresh fetched: the records the sources answered with, and the
/// collection rails that answered at all — an empty page is an answer, a source
/// that could not be asked (offline, no key) is not.
pub struct Fetched {
    pub movies: Vec<MovieItem>,
    pub answered: Vec<String>,
}

/// Rewrites the root from the records `fetch` returns, keeping the catalog pages
/// fetched before: browsing deeper than the rails survives a refresh.
///
/// A collection rail is resolved whole every time, so its titles outlive the
/// refresh under one rule: a rail the person turned off (`rails`, the enabled
/// collection ids) takes its titles with it, and so does a title the rail no
/// longer holds when it did answer — the rail follows its page. A rail that could
/// not be asked this time (offline, no key) keeps its last good titles instead of
/// emptying.
pub fn refresh<F>(root: &Path, rails: &[String], fetch: F) -> Result<()>
where
    F: FnOnce() -> Result<Fetched>,
{
    let Fetched { mut movies, answered } = fetch()?;
    for item in stored(root).into_iter().filter(|item| kept(item, rails, &answered)) {
        if !movies.iter().any(|movie| same(movie, &item)) {
            movies.push(item);
        }
    }
    write_root(root, &movies)
}

/// Whether a stored record outlives the refresh that is writing now: a catalogue
/// page always, a collection rail's title only while the rail is on and could not
/// be asked — one that answered has already brought back what it still holds.
fn kept(item: &MovieItem, rails: &[String], answered: &[String]) -> bool {
    let Some(mark) = item.catalog.as_deref() else { return false };
    match mark.strip_prefix(MARK_PREFIX) {
        Some(id) => rails.iter().any(|rail| rail == id) && !answered.iter().any(|rail| rail == id),
        None => true,
    }
}

/// Merges a page of a catalog into the root's library, leaving the rails and
/// the pages before it as they are. Returns how many items were new.
pub fn append(root: &Path, movies: &[MovieItem]) -> Result<usize> {
    let mut all = stored(root);
    let before = all.len();
    for movie in movies {
        if !all.iter().any(|seen| same(seen, movie)) {
            all.push(movie.clone());
        }
    }
    let added = all.len() - before;
    if added > 0 {
        write_root(root, &all)?;
    }
    Ok(added)
}

/// Replaces the catalog pages of one kind with `movies`, in a single write: a
/// tab shows one feed at a time, so a changed catalog, order or filter must not
/// leave an earlier feed's pages behind. The rails and the other kind stay
/// untouched. Returns how many items were new.
pub fn replace(root: &Path, kind: &str, movies: &[MovieItem]) -> Result<usize> {
    let prefix = format!("{kind}:");
    let mut all: Vec<MovieItem> =
        stored(root).into_iter().filter(|item| item.catalog.as_deref().is_none_or(|feed| !feed.starts_with(&prefix))).collect();
    let before = all.len();
    for movie in movies {
        if !all.iter().any(|seen| same(seen, movie)) {
            all.push(movie.clone());
        }
    }
    let added = all.len() - before;
    write_root(root, &all)?;
    Ok(added)
}

/// Whether two items are the same title (the id alone is not enough: a film and
/// a series may share one).
fn same(a: &MovieItem, b: &MovieItem) -> bool {
    a.id == b.id && a.kind == b.kind
}

/// The items a root holds now, read back from the layouts [`write_root`] wrote.
fn stored(root: &Path) -> Vec<MovieItem> {
    let paths = DataPaths::at(root.to_path_buf());
    let games: Value = json_store::read(&paths.sideload_library()).unwrap_or(Value::Null);
    let metadata: Value = json_store::read(&paths.metadata()).unwrap_or(Value::Null);
    let timestamps: Value = json_store::read(&paths.timestamps()).unwrap_or(Value::Null);
    let Some(games) = games.get("games").and_then(Value::as_array) else { return Vec::new() };
    games.iter().filter_map(|game| stored_item(game, &metadata, &timestamps)).collect()
}

fn stored_item(game: &Value, metadata: &Value, timestamps: &Value) -> Option<MovieItem> {
    let extra = game.get("extra")?;
    let id = extra.get("watchId")?.as_str()?.to_owned();
    let runner = game.get("runner").and_then(Value::as_str).unwrap_or("stremio");
    let app = game.get("app_name").and_then(Value::as_str)?;
    let details = metadata.get("games").and_then(|games| games.get(format!("{runner}:{app}")));
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    let stamp = timestamps.get(app);
    Some(MovieItem {
        id,
        kind: extra.get("stremioType").and_then(Value::as_str).unwrap_or("movie").to_owned(),
        name: game.get("title").and_then(Value::as_str).unwrap_or_default().to_owned(),
        year: text(details.and_then(|details| details.get("releaseDate"))),
        poster: text(game.get("art_cover")),
        background: text(game.get("art_background")),
        logo: text(game.get("art_logo")).filter(|logo| !logo.is_empty()),
        description: text(game.get("description")),
        genres: details
            .and_then(|details| details.get("genres"))
            .and_then(Value::as_array)
            .map(|genres| genres.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default(),
        top: extra.get("stremioTop").and_then(Value::as_bool).unwrap_or(false),
        catalog: text(extra.get(CATALOG_KEY)),
        last_watched: text(stamp.and_then(|stamp| stamp.get("lastPlayed"))),
        minutes_played: stamp.and_then(|stamp| stamp.get("totalPlayed")).and_then(Value::as_f64).unwrap_or(0.0),
        provider: runner.to_owned(),
        ..MovieItem::default()
    })
}

/// Writes the exact layouts `Library::load_with` reads. Every other file is
/// absent, and `json_store` treats a missing file as its default.
pub fn write_root(root: &Path, movies: &[MovieItem]) -> Result<()> {
    let paths = DataPaths::at(root.to_path_buf());
    let games: Vec<Value> = movies.iter().map(game_json).collect();
    json_store::write(&paths.sideload_library(), &json!({"games": games})).context("writing the movie list")?;

    let mut local = Map::new();
    let mut metadata = Map::new();
    let mut art = Map::new();
    for movie in movies {
        let name = app_name(movie);
        local.insert(name.clone(), local_meta(movie, &name));
        // `collectionMetadataKey` / `collectionArtKey`: `runner:appName`, and
        // `appName` already carries the provider.
        let key = format!("{}:{}", provider(movie), name);
        metadata.insert(key.clone(), movie_metadata(movie, &name));
        art.insert(key, movie_art(movie));
    }
    json_store::write(&paths.local_meta(), &json!({"games": local}))?;
    json_store::write(&paths.metadata(), &json!({"games": metadata}))?;
    json_store::write(&paths.art(), &json!({"games": art}))?;

    // `store/timestamp.json` makes "Continue playing"/"Continue watching"
    // read Stremio's real timestamps instead of an empty list.
    let mut timestamps = Map::new();
    for movie in movies {
        if movie.last_watched.is_some() || movie.minutes_played > 0.0 {
            timestamps.insert(
                app_name(movie),
                json!({
                    "lastPlayed": &movie.last_watched,
                    "totalPlayed": movie.minutes_played,
                }),
            );
        }
    }
    json_store::write(&paths.timestamps(), &Value::Object(timestamps))?;
    Ok(())
}

/// The provider that owns an item; an item without one is Stremio's.
fn provider(movie: &MovieItem) -> &str {
    if movie.provider.is_empty() {
        "stremio"
    } else {
        movie.provider.as_str()
    }
}

/// The Heroic `app_name` of an item: provider-prefixed, so two providers with
/// the same id never share a local record or a timestamp.
fn app_name(movie: &MovieItem) -> String {
    format!("{}:{}", provider(movie), movie.id)
}

fn game_json(movie: &MovieItem) -> Value {
    let kind = if movie.kind.is_empty() { "movie" } else { movie.kind.as_str() };
    let provider = provider(movie);
    let poster = movie.poster.clone();
    json!({
        "runner": provider,
        "app_name": app_name(movie),
        "title": &movie.name,
        "art_cover": poster.clone(),
        "art_square": poster,
        "art_background": movie.background.clone().or_else(|| movie.poster.clone()),
        "art_logo": &movie.logo,
        "description": &movie.description,
        "is_installed": true,
        "install": {"platform": "movie"},
        "extra": {
            "stremioType": kind,
            "stremioTop": movie.top,
            CATALOG_KEY: &movie.catalog,
            "provider": provider,
            "watchId": &movie.id,
        },
    })
}

fn local_meta(movie: &MovieItem, name: &str) -> Value {
    let provider = provider(movie);
    json!({
        "appName": name,
        "runner": provider,
        "source": provider,
        "launchKind": "unavailable",
        "title": &movie.name,
    })
}

fn movie_metadata(movie: &MovieItem, name: &str) -> Value {
    json!({
        "appName": name,
        "runner": provider(movie),
        "title": &movie.name,
        "description": &movie.description,
        "releaseDate": &movie.year,
        "genres": &movie.genres,
        "coverUrl": &movie.poster,
        "heroUrl": &movie.background,
    })
}

fn movie_art(movie: &MovieItem) -> Value {
    json!({
        "coverUrl": &movie.poster,
        "heroUrl": &movie.background,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::Library;

    fn movie(id: &str, name: &str) -> MovieItem {
        MovieItem {
            id: id.into(),
            kind: "movie".into(),
            name: name.into(),
            year: Some("2026".into()),
            poster: Some(format!("https://img/{id}/poster.jpg")),
            background: Some(format!("https://img/{id}/hero.jpg")),
            description: Some("A plot".into()),
            genres: vec!["Drama".into()],
            ..MovieItem::default()
        }
    }

    #[test]
    fn movies_load_through_the_existing_library() {
        let dir = tempfile::tempdir().unwrap();
        write_root(dir.path(), &[movie("tt1", "A Film")]).unwrap();
        let lib = Library::load_with(&DataPaths::at(dir.path().to_path_buf()), true).unwrap();
        assert_eq!(lib.games.len(), 1);
        let game = &lib.games[0];
        assert_eq!(game.runner, "stremio");
        assert_eq!(game.app_name, "stremio:tt1");
        assert_eq!(game.title, "A Film");
        assert_eq!(game.game.art_cover.as_deref(), Some("https://img/tt1/poster.jpg"));
        assert_eq!(game.metadata.as_deref().and_then(|m| m.description.as_deref()), Some("A plot"));
        assert_eq!(game.metadata.as_deref().map(|m| m.genres.clone()), Some(vec!["Drama".to_owned()]));
    }

    #[test]
    fn a_catalog_page_is_merged_and_survives_a_rails_refresh() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_root(root, &[movie("tt1", "A Film")]).unwrap();

        let mut page = movie("tt2", "Another Film");
        page.catalog = Some("movie:top:Popular".into());
        let added = append(root, &[page, movie("tt1", "A Film")]).unwrap();
        assert_eq!(added, 1, "a title already there is not added again");

        // A rails refresh replaces the rails and keeps the page.
        refresh(root, &[], || Ok(Fetched { movies: vec![movie("tt3", "New Film")], answered: Vec::new() })).unwrap();
        let items = stored(root);
        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["tt3", "tt2"]);
        assert!(items.iter().any(|item| item.id == "tt2" && item.catalog.is_some()), "the page is still marked");
        assert_eq!(items[1].year.as_deref(), Some("2026"), "the page keeps its metadata");
    }

    #[test]
    fn a_collection_rail_follows_its_page_and_keeps_what_it_could_not_ask() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_root(root, &[movie("tt1", "A Film")]).unwrap();
        let mut in_rail = movie("tt2", "Top Rated Film");
        in_rail.catalog = Some(heronite_watch_core::collections::mark("tmdb:top-rated:movie"));
        append(root, &[in_rail]).unwrap();

        // The rail is on and this refresh could not ask (offline): its last good
        // titles stay, which is what the tab draws while the source is away.
        let rail = ["tmdb:top-rated:movie".to_owned()];
        let nothing = || Ok(Fetched { movies: vec![movie("tt1", "A Film")], answered: Vec::new() });
        refresh(root, &rail, nothing).unwrap();
        assert!(stored(root).iter().any(|item| item.id == "tt2"), "an unreachable rail keeps its titles");

        // The rail answered, and its page no longer holds the title: the rail
        // follows the page rather than accumulating what it once showed.
        let answered = || Ok(Fetched { movies: vec![movie("tt1", "A Film")], answered: vec!["tmdb:top-rated:movie".to_owned()] });
        refresh(root, &rail, answered).unwrap();
        assert!(!stored(root).iter().any(|item| item.id == "tt2"), "a title off the page leaves the rail");

        // Turned off: the library no longer holds its titles either, whether or
        // not the source answered.
        let mut popular = movie("tt4", "Another Rail Film");
        popular.catalog = Some(heronite_watch_core::collections::mark("tmdb:popular:movie"));
        append(root, &[popular]).unwrap();
        refresh(root, &[], || Ok(Fetched { movies: vec![movie("tt1", "A Film")], answered: Vec::new() })).unwrap();
        let ids: Vec<String> = stored(root).into_iter().map(|item| item.id).collect();
        assert_eq!(ids, ["tt1"], "a rail the person turned off leaves no titles behind");
    }

    #[test]
    fn replacing_a_feed_drops_only_its_pages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_root(root, &[movie("tt1", "A Film")]).unwrap();
        let mut old = movie("tt2", "Old Page");
        old.catalog = Some("movie:top:Popular".into());
        let mut other = movie("tt3", "Other Kind");
        other.kind = "series".into();
        other.catalog = Some("series:top:Popular".into());
        append(root, &[old, other]).unwrap();

        let mut fresh = movie("tt9", "New Page");
        fresh.catalog = Some("movie:imdbRating:Rating".into());
        let added = replace(root, "movie", &[fresh]).unwrap();
        assert_eq!(added, 1);

        let items = stored(root);
        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&"tt1"), "the rails stay: {ids:?}");
        assert!(ids.contains(&"tt9"), "the new page is there: {ids:?}");
        assert!(!ids.contains(&"tt2"), "the previous feed's page is gone: {ids:?}");
        assert!(ids.contains(&"tt3"), "the other kind is untouched: {ids:?}");
    }

    #[test]
    fn a_merge_keeps_the_progress_of_what_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut watched = movie("tt1", "A Film");
        watched.last_watched = Some("2026-11-12T20:00:00Z".into());
        watched.minutes_played = 42.0;
        write_root(root, &[watched]).unwrap();

        let mut page = movie("tt2", "Another Film");
        page.catalog = Some("movie:top:Popular".into());
        append(root, &[page]).unwrap();
        let items = stored(root);
        let watched = items.iter().find(|item| item.id == "tt1").expect("the title is still there");
        assert_eq!(watched.last_watched.as_deref(), Some("2026-11-12T20:00:00Z"));
        assert_eq!(watched.minutes_played, 42.0);
    }

    #[test]
    fn the_projection_keys_carry_the_provider() {
        let dir = tempfile::tempdir().unwrap();
        write_root(dir.path(), &[movie("tt1", "A Film")]).unwrap();
        let paths = DataPaths::at(dir.path().to_path_buf());

        let local: Value = json_store::read(&paths.local_meta()).unwrap();
        assert!(local["games"]["stremio:tt1"].is_object(), "{local}");
        let metadata: Value = json_store::read(&paths.metadata()).unwrap();
        assert!(metadata["games"]["stremio:stremio:tt1"].is_object(), "{metadata}");
        assert!(metadata["games"]["tt1"].is_null(), "{metadata}");

        // Timestamps stay a bare object, not `{"games": …}`, and stay empty
        // without a last watch or played minutes.
        let timestamps: Value = json_store::read(&paths.timestamps()).unwrap();
        assert!(timestamps["stremio:tt1"].is_null(), "{timestamps}");

        let library: Value = json_store::read(&paths.sideload_library()).unwrap();
        assert_eq!(library["games"][0]["runner"], "stremio");
        assert_eq!(library["games"][0]["app_name"], "stremio:tt1");
        assert_eq!(library["games"][0]["extra"]["watchId"], "tt1");
        assert_eq!(library["games"][0]["extra"]["stremioType"], "movie");
        assert_eq!(library["games"][0]["install"]["platform"], "movie");
    }
}

//! Watch items as a `Library` the existing Collection screens already read.
//!
//! The proxy writes a regenerable Heroic-format root under the app cache; the
//! facade loads it with `Library::load_with`. No Collection, console or
//! game-page code needs to know which provider the records came from.

use std::path::Path;

use anyhow::{Context, Result};
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_watch_core::types::MovieItem;
use serde_json::{json, Map, Value};

/// Where the proxy keeps its synthetic library.
pub const ROOT: &str = "watch-library";

/// Rewrites the root from the items `fetch` returns.
pub fn refresh<F>(root: &Path, fetch: F) -> Result<()>
where
    F: FnOnce() -> Result<Vec<MovieItem>>,
{
    let movies = fetch()?;
    write_root(root, &movies)
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

//! Stremio movies as a `Library` the existing Collection screens already read.
//!
//! The proxy writes a regenerable Heroic-format root under the app cache and
//! calls `Library::load_with`. No Collection, console or game-page code needs
//! to know that the records came from Stremio.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use heronite_data::paths::AppPaths;
use heronite_data::{json_store, DataPaths, Library};
use serde_json::{json, Map, Value};

use crate::addon::AddonClient;
use crate::datastore::{continue_watching, DatastoreClient};
use crate::endpoints::Endpoints;
use crate::store;
use crate::types::MovieItem;
use crate::UreqTransport;

const CINEMETA: &str = "https://v3-cinemeta.strem.io/manifest.json";
/// Where the proxy keeps its synthetic library.
pub const ROOT: &str = "movies-library";

/// The movies Collection, refreshed from Stremio when possible. A provider
/// failure keeps the last good library instead of emptying the screen.
pub fn library(app: &AppPaths, include_hidden: bool) -> Result<Library> {
    let root = app.cache.join(ROOT);
    if let Err(error) = refresh(&root, app) {
        log::warn!("movies provider: {error:#}; keeping the last library");
    }
    Library::load_with(&DataPaths::at(root), include_hidden)
}

pub fn refresh(root: &Path, app: &AppPaths) -> Result<()> {
    let movies = fetch(app)?;
    write_root(root, &movies)
}

/// The linked account's films, then Cinemeta's Top. Series are out of scope in
/// v1; the catalog is films only.
pub fn fetch(app: &AppPaths) -> Result<Vec<MovieItem>> {
    let http = UreqTransport::new();
    let endpoints = Endpoints::default();
    let mut movies = Vec::new();
    if let Some(key) = store::read_auth(app)? {
        let client = DatastoreClient::new(&http, &endpoints, key);
        for item in continue_watching(&client.get_all()?) {
            if item.kind == "movie" {
                movies.push(movie_from_library(item));
            }
        }
    }
    let mut addon = AddonClient::new(&http, CINEMETA)?;
    addon.manifest()?;
    movies.extend(addon.catalog("movie", "top", &[])?.into_iter().map(|mut movie| {
        movie.top = true;
        movie
    }));
    let mut seen = HashSet::new();
    movies.retain(|movie| seen.insert(movie.id.clone()));
    Ok(movies)
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
        local.insert(movie.id.clone(), local_meta(movie));
        metadata.insert(format!("stremio:{}", movie.id), movie_metadata(movie));
        art.insert(format!("stremio:{}", movie.id), movie_art(movie));
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
                movie.id.clone(),
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

fn movie_from_library(item: &crate::datastore::LibraryItem) -> MovieItem {
    MovieItem {
        id: item.id.clone(),
        kind: item.kind.clone(),
        name: item.name.clone(),
        poster: item.poster.clone(),
        last_watched: item.state.last_watched.map(|date| date.to_rfc3339()),
        minutes_played: item.state.time_watched as f64 / 60_000.0,
        ..MovieItem::default()
    }
}

fn game_json(movie: &MovieItem) -> Value {
    let kind = if movie.kind.is_empty() { "movie" } else { movie.kind.as_str() };
    let poster = movie.poster.clone();
    json!({
        "runner": "stremio",
        "app_name": &movie.id,
        "title": &movie.name,
        "art_cover": poster.clone(),
        "art_square": poster,
        "art_background": movie.background.clone().or_else(|| movie.poster.clone()),
        "art_logo": &movie.logo,
        "description": &movie.description,
        "is_installed": true,
        "install": {"platform": "movie"},
        "extra": {"stremioType": kind, "stremioTop": movie.top},
    })
}

fn local_meta(movie: &MovieItem) -> Value {
    json!({
        "appName": &movie.id,
        "runner": "stremio",
        "source": "stremio",
        "launchKind": "unavailable",
        "title": &movie.name,
    })
}

fn movie_metadata(movie: &MovieItem) -> Value {
    json!({
        "appName": &movie.id,
        "runner": "stremio",
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
        assert_eq!(game.title, "A Film");
        assert_eq!(game.game.art_cover.as_deref(), Some("https://img/tt1/poster.jpg"));
        assert_eq!(game.metadata.as_deref().and_then(|m| m.description.as_deref()), Some("A plot"));
        assert_eq!(game.metadata.as_deref().map(|m| m.genres.clone()), Some(vec!["Drama".to_owned()]));
    }
}

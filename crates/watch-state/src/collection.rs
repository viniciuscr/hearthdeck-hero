//! Watch titles as a `Library` the existing Collection screens already read.
//!
//! The proxy writes a regenerable Heroic-format root under the app cache; the
//! facade loads it with `Library::load_with`. No Collection, console or
//! game-page code needs to know which provider the records came from.
//!
//! The projection's currency is [`Title`] (`docs/WATCH-TITLES.md` §5): it takes
//! titles, writes the frozen keys, and reads them back as titles. The one wire
//! shape — [`MovieItem`] — is the record between the two, so the decode lives
//! here, beside the encode that has to match it.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::SecondsFormat;
use heronite_data::copies::CopiesFile;
use heronite_data::json_store;
use heronite_data::DataPaths;
use heronite_watch_core::title::{ProviderId, RailMark, Title, IMPORTED_ADDON};
use heronite_watch_core::types::MovieItem;
use serde_json::{json, Map, Value};

/// Where the proxy keeps its synthetic library.
pub const ROOT: &str = "watch-library";

/// The mark a catalog page leaves in its records, so a rails refresh keeps them.
pub const CATALOG_KEY: &str = "watchCatalog";

/// Where the provider left off, how long the title is and whether it is
/// finished: the three fields `timeWatched` does not say (it stays 0 for a title
/// that was only started). Kept in the record's `extra`, the projection's own
/// namespace, so a reloaded `CollectionGame` still carries them.
pub const POSITION_KEY: &str = "watchPosition";
pub const DURATION_KEY: &str = "watchDuration";
pub const FINISHED_KEY: &str = "watchFinished";
/// The day the title came out and how long it runs: a catalogue has no added
/// date, so "New" measures the first, and the facts line shows the second.
pub const RELEASED_KEY: &str = "watchReleased";
pub const RUNTIME_KEY: &str = "watchRuntime";
/// The catalogue or provider that listed the title: the film page names it
/// instead of the stream provider every record is stamped with.
pub const SOURCE_KEY: &str = "watchSource";
/// What the last stream probe found (`true` a stream, `false` none); absent
/// when no probe is known or the answer aged out.
pub const STREAMS_KEY: &str = "watchStreams";

/// What a rails refresh fetched: the titles the sources answered with, and the
/// collection rails that answered at all — an empty page is an answer, a source
/// that could not be asked (offline, no key) is not.
pub struct Fetched {
    pub titles: Vec<Title>,
    pub answered: Vec<String>,
}

/// Rewrites the root from the titles `fetch` returns, keeping the catalog pages
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
    let Fetched { mut titles, answered } = fetch()?;
    for kept in stored(root).into_iter().filter(|title| kept(title, rails, &answered)) {
        merge(&mut titles, kept);
    }
    write_root(root, &titles)
}

/// Whether a stored title outlives the refresh that is writing now: a catalogue
/// page always, a collection rail's title only while the rail is on and could not
/// be asked — one that answered has already brought back what it still holds.
fn kept(title: &Title, rails: &[String], answered: &[String]) -> bool {
    match title.rails.first() {
        // A title no rail marks is what the sources answered with just now.
        None => false,
        Some(RailMark::Feed { .. }) => true,
        Some(RailMark::Collection(id)) => rails.iter().any(|rail| rail == id) && !answered.iter().any(|rail| rail == id),
    }
}

/// Merges a page of a catalog into the root's library, leaving the rails and
/// the pages before it as they are. Returns how many items were new.
pub fn append(root: &Path, titles: &[Title]) -> Result<usize> {
    let mut all = stored(root);
    let before = all.len();
    for title in titles {
        merge(&mut all, title.clone());
    }
    let added = all.len() - before;
    if added > 0 {
        write_root(root, &all)?;
    }
    Ok(added)
}

/// Replaces the catalog pages of one kind with `titles`, in a single write: a
/// tab shows one feed at a time, so a changed catalog, order or filter must not
/// leave an earlier feed's pages behind. The rails and the other kind stay
/// untouched. Returns how many items were new.
pub fn replace(root: &Path, kind: &str, titles: &[Title]) -> Result<usize> {
    let prefix = format!("{kind}:");
    let mut all: Vec<Title> = stored(root)
        .into_iter()
        .filter(|title| match title.rails.first() {
            Some(RailMark::Feed { catalog, .. }) => !catalog.starts_with(&prefix),
            _ => true,
        })
        .collect();
    let before = all.len();
    for title in titles {
        merge(&mut all, title.clone());
    }
    let added = all.len() - before;
    write_root(root, &all)?;
    Ok(added)
}

/// Adds a title, or folds it into the one already there.
fn merge(titles: &mut Vec<Title>, title: Title) {
    match titles.iter().position(|held| held.key == title.key) {
        Some(at) => {
            // The two share a key, so the merge cannot refuse the observation.
            if titles[at].merge(title).is_some() {
                log::error!("watch projection: a title changed key while merging");
            }
        }
        None => titles.push(title),
    }
}

/// The titles a root holds now, read back from the layouts [`write_root`] wrote.
///
/// A corrupt file is a fault, not an empty library: it is said in the log, and
/// the refresh keeps what the fetchers brought rather than writing over pages
/// the person had browsed.
fn stored(root: &Path) -> Vec<Title> {
    let paths = DataPaths::at(root.to_path_buf());
    let games: Value = read_or_warn(&paths.sideload_library());
    let metadata: Value = read_or_warn(&paths.metadata());
    let timestamps: Value = read_or_warn(&paths.timestamps());
    let Some(games) = games.get("games").and_then(Value::as_array) else { return Vec::new() };
    let mut titles = Vec::new();
    for game in games {
        let Some(item) = stored_item(game, &metadata, &timestamps) else { continue };
        // One title played by several providers is several records: fold them
        // back into one, the way `write_root` split them.
        merge(&mut titles, Title::of_item(&item));
    }
    titles
}

/// The titles a root holds now.
///
/// Public so the projection's other reader — `title_of_game`, over a loaded
/// `Library` — can be held to it: the two decode the same keys, and a test pits
/// them against each other.
pub fn titles(root: &Path) -> Vec<Title> {
    stored(root)
}

/// A projection file, or an empty document when it is missing — and a log line
/// when it is there but unreadable, so a broken file does not read as an empty
/// library.
fn read_or_warn(path: &Path) -> Value {
    match json_store::read_value(path) {
        Ok(value) => value,
        Err(error) => {
            log::warn!("watch projection {}: {error:#}", path.display());
            Value::Null
        }
    }
}

/// The record a written game is: the wire shape, decoded.
fn stored_item(game: &Value, metadata: &Value, timestamps: &Value) -> Option<MovieItem> {
    let extra = game.get("extra")?;
    let id = extra.get("watchId")?.as_str()?.to_owned();
    let runner = game.get("runner").and_then(Value::as_str).unwrap_or("stremio");
    let app = game.get("app_name").and_then(Value::as_str)?;
    let details = metadata.get("games").and_then(|games| games.get(format!("{runner}:{app}")));
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    let stamp = timestamps.get(app);
    // An imported record names no provider: `runner` is the owner there, not a
    // source that can play it.
    let imported = extra.get("imported").and_then(Value::as_bool).unwrap_or(runner == IMPORTED_ADDON);
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
        runtime: text(extra.get(RUNTIME_KEY)),
        released: text(extra.get(RELEASED_KEY)),
        position_ms: extra.get(POSITION_KEY).and_then(Value::as_u64).unwrap_or(0),
        duration_ms: extra.get(DURATION_KEY).and_then(Value::as_u64).unwrap_or(0),
        finished: extra.get(FINISHED_KEY).and_then(Value::as_bool).unwrap_or(false),
        last_watched: text(stamp.and_then(|stamp| stamp.get("lastPlayed"))),
        minutes_played: stamp.and_then(|stamp| stamp.get("totalPlayed")).and_then(Value::as_f64).unwrap_or(0.0),
        provider: if imported { String::new() } else { runner.to_owned() },
        addon: if imported { IMPORTED_ADDON.to_owned() } else { text(extra.get(SOURCE_KEY)).unwrap_or_default() },
    })
}

/// The record a title becomes, played by `provider` — or the placeholder record
/// when no provider plays it: the inverse of [`stored_item`], and the one place
/// the projection's shapes are written.
///
/// `with_watch` puts the title's watch state on this record. Only one record per
/// title carries it: upstream's copies total sums every copy, so writing the
/// same minutes on each would double the "N in all" figure on the grouped card.
fn record_of(title: &Title, provider: Option<&ProviderId>, with_watch: bool) -> MovieItem {
    // The catalogue that listed it, even when a collection mark names it first:
    // the rail source is what a card says the title comes from.
    let feed = title.rails.iter().find_map(|mark| match mark {
        RailMark::Feed { source, .. } => Some(source.clone()),
        RailMark::Collection(_) => None,
    });
    let watched = title.watched.as_ref().filter(|_| with_watch);
    MovieItem {
        id: title.key.id.as_str().to_owned(),
        kind: title.key.kind.as_str().to_owned(),
        name: title.name.clone(),
        year: title.release.clone(),
        poster: title.art.poster.clone(),
        background: title.art.background.clone(),
        logo: title.art.logo.clone(),
        description: title.description.clone(),
        runtime: title.runtime.clone(),
        released: title.released.clone(),
        genres: title.genres.clone(),
        top: title.top,
        catalog: title.rails.first().map(RailMark::as_wire),
        last_watched: watched.and_then(|seen| seen.at).map(stamp),
        minutes_played: watched.map_or(0.0, |seen| seen.minutes),
        position_ms: watched.map_or(0, |seen| seen.position_ms),
        duration_ms: watched.map_or(0, |seen| seen.duration_ms),
        finished: watched.is_some_and(|seen| seen.finished),
        provider: provider.map(|id| id.as_str().to_owned()).unwrap_or_default(),
        addon: match provider {
            Some(_) => feed.unwrap_or_default(),
            None => IMPORTED_ADDON.to_owned(),
        },
    }
}

/// Writes the exact layouts `Library::load_with` reads. Every other file is
/// absent, and `json_store` treats a missing file as its default.
///
/// A title the person can play from more than one provider is written once per
/// provider — each record names the one that plays it — and the records are
/// linked in `local_library/copies.json`, upstream's own "one game, several
/// stores", so the Collection collapses them to one card with the copies badge.
/// The person's own grouping wins (see [`write_copies`]).
///
/// The wire key is `{owner}:{id}` and carries no kind, so a film and a series
/// sharing an id collide: the first title wins, and the second is left out
/// (§5.1a). Changing the key would break the frozen names.
pub fn write_root(root: &Path, titles: &[Title]) -> Result<()> {
    let paths = DataPaths::at(root.to_path_buf());
    let mut seen = HashSet::new();
    let mut records: Vec<MovieItem> = Vec::new();
    let mut groups: Vec<Vec<String>> = Vec::new();
    for title in titles {
        let mut group = Vec::new();
        if title.providers.is_empty() {
            let record = record_of(title, None, true);
            if seen.insert(app_name(&record)) {
                group.push(collection_key(&record));
                records.push(record);
            }
        } else {
            // One record per provider, and the watch state on the first of them.
            for (i, provider) in title.providers.iter().enumerate() {
                let record = record_of(title, Some(provider), i == 0);
                if seen.insert(app_name(&record)) {
                    group.push(collection_key(&record));
                    records.push(record);
                }
            }
        }
        if group.len() > 1 {
            groups.push(group);
        }
    }

    let games: Vec<Value> = {
        // What the stream probes found, read once: a catalogue lists a title,
        // only a stream answer says someone can play it.
        let probes = crate::streams::read(root);
        records.iter().map(|record| game_json(record, &probes)).collect()
    };
    json_store::write(&paths.sideload_library(), &json!({"games": games})).context("writing the movie list")?;

    let mut local = Map::new();
    let mut metadata = Map::new();
    let mut art = Map::new();
    for record in &records {
        let name = app_name(record);
        local.insert(name.clone(), local_meta(record, &name));
        // `collectionMetadataKey` / `collectionArtKey`: `runner:appName`, and
        // `appName` already carries the provider.
        let key = format!("{}:{}", owner(record), name);
        metadata.insert(key.clone(), movie_metadata(record, &name));
        art.insert(key, movie_art(record));
    }
    json_store::write(&paths.local_meta(), &json!({"games": local}))?;
    json_store::write(&paths.metadata(), &json!({"games": metadata}))?;
    json_store::write(&paths.art(), &json!({"games": art}))?;

    // `store/timestamp.json` makes "Continue playing"/"Continue watching"
    // read Stremio's real timestamps instead of an empty list.
    let mut timestamps = Map::new();
    for record in &records {
        if record.last_watched.is_some() || record.minutes_played > 0.0 {
            timestamps.insert(
                app_name(record),
                json!({
                    "lastPlayed": &record.last_watched,
                    "totalPlayed": record.minutes_played,
                }),
            );
        }
    }
    json_store::write(&paths.timestamps(), &Value::Object(timestamps))?;
    write_copies(&paths, &groups)
}

/// The key `CollectionGame` gives a record: `runner_appName`, which is what
/// `copies.json` holds.
fn collection_key(record: &MovieItem) -> String {
    format!("{}_{}", owner(record), app_name(record))
}

/// Links the records of each title, so upstream's copies UI draws one card and
/// offers "Play with \<provider\>".
///
/// Read-modify-write on the file the person also edits: their `preferred` list
/// and their other links are kept, and a pair they ungrouped is never linked
/// again. A title that loses a provider keeps its old link — `copies.json` keeps
/// keys the library no longer holds and rejoins them when they return.
fn write_copies(paths: &DataPaths, groups: &[Vec<String>]) -> Result<()> {
    if groups.is_empty() {
        return Ok(());
    }
    json_store::update_if_changed::<CopiesFile, _>(&paths.copies(), |file| {
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            let ungrouped = file.apart.iter().any(|[a, b]| group.contains(a) && group.contains(b));
            if ungrouped {
                continue;
            }
            // A title that gained a provider supersedes the smaller link it had
            // before: replace any link this group contains rather than adding a
            // second, overlapping one to a file the person owns.
            file.links.retain(|link| !link.iter().all(|key| group.contains(key)));
            file.links.push(group.clone());
        }
        Ok(())
    })
    .context("writing the copy groups")
}

/// The provider's own stamp, in the shape the projection sorts by: UTC, and
/// written with milliseconds so a lexicographic order is a chronological one
/// (`docs/WATCH-TITLES.md` §9.9).
fn stamp(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The provider that plays a title, for the records that name one.
///
/// One record carries one provider — the smallest id, so two runs agree — and a
/// title no provider plays names Stremio, as the record always has.
fn provider(record: &MovieItem) -> &str {
    if record.provider.is_empty() {
        "stremio"
    } else {
        record.provider.as_str()
    }
}

/// Who "owns" an item: its provider, or `imported` for a placeholder nothing here
/// can play. Kept out of Stremio's namespace on purpose — an imported film must
/// not borrow a provider's id or its play state.
fn owner(record: &MovieItem) -> &str {
    if record.addon == IMPORTED_ADDON {
        IMPORTED_ADDON
    } else {
        provider(record)
    }
}

/// The Heroic `app_name` of an item: owner-prefixed, so two providers with the
/// same id never share a local record or a timestamp.
fn app_name(record: &MovieItem) -> String {
    format!("{}:{}", owner(record), record.id)
}

fn game_json(record: &MovieItem, probes: &std::collections::HashMap<String, crate::streams::Probe>) -> Value {
    let kind = if record.kind.is_empty() { "movie" } else { record.kind.as_str() };
    let owner = owner(record);
    let imported = record.addon == IMPORTED_ADDON;
    // Absent when never asked (or the answer aged out): the screens say nothing
    // about a title they have not tried.
    let streams = match crate::streams::availability(probes, provider(record), kind, &record.id) {
        crate::streams::Availability::Available => Some(true),
        crate::streams::Availability::Unavailable => Some(false),
        crate::streams::Availability::Unknown => None,
    };
    let poster = record.poster.clone();
    json!({
        "runner": owner,
        "app_name": app_name(record),
        "title": &record.name,
        "art_cover": poster.clone(),
        "art_square": poster,
        "art_background": record.background.clone().or_else(|| record.poster.clone()),
        "art_logo": &record.logo,
        "description": &record.description,
        // Repurposed for Watch: "installed" means a stream source can play it.
        // A placeholder has none, which is exactly what the tiles grey on.
        "is_installed": !imported,
        "install": {"platform": "movie"},
        "extra": {
            "stremioType": kind,
            "stremioTop": record.top,
            CATALOG_KEY: &record.catalog,
            "provider": owner,
            "watchId": &record.id,
            "imported": imported,
            RELEASED_KEY: &record.released,
            RUNTIME_KEY: &record.runtime,
            POSITION_KEY: record.position_ms,
            DURATION_KEY: record.duration_ms,
            FINISHED_KEY: record.finished,
            // Where the title was listed, and what the last probe found: a
            // null `watchStreams` is "never asked".
            SOURCE_KEY: &record.addon,
            STREAMS_KEY: streams,
        },
    })
}

fn local_meta(record: &MovieItem, name: &str) -> Value {
    let provider = provider(record);
    json!({
        "appName": name,
        "runner": provider,
        "source": provider,
        "launchKind": "unavailable",
        "title": &record.name,
    })
}

fn movie_metadata(record: &MovieItem, name: &str) -> Value {
    json!({
        "appName": name,
        "runner": provider(record),
        "title": &record.name,
        "description": &record.description,
        "releaseDate": &record.year,
        "genres": &record.genres,
        "coverUrl": &record.poster,
        "heroUrl": &record.background,
    })
}

fn movie_art(record: &MovieItem) -> Value {
    json!({
        "coverUrl": &record.poster,
        "heroUrl": &record.background,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_data::Library;
    use heronite_watch_core::title::{Art, ProviderId, TitleId, TitleKey, TitleKind};

    fn provider() -> ProviderId {
        ProviderId::declared("stremio")
    }

    /// A title a source answers with: a film, one provider, its fields filled.
    fn title(id: &str, name: &str) -> Title {
        let mut title = Title::new(TitleKey::of_source("movie", id, &provider()), name).plays(provider());
        title.release = Some("2026".into());
        title.art = Art {
            poster: Some(format!("https://img/{id}/poster.jpg")),
            background: Some(format!("https://img/{id}/hero.jpg")),
            logo: None,
        };
        title.description = Some("A plot".into());
        title.genres = vec!["Drama".into()];
        title
    }

    fn ids(titles: &[Title]) -> Vec<String> {
        titles.iter().map(|title| title.key.id.as_str().to_owned()).collect()
    }

    #[test]
    fn titles_load_through_the_existing_library() {
        let dir = tempfile::tempdir().unwrap();
        write_root(dir.path(), &[title("tt1", "A Film")]).unwrap();
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
        write_root(root, &[title("tt1", "A Film")]).unwrap();

        let mut page = title("tt2", "Another Film");
        page.rails = vec![RailMark::feed("cinemeta", "movie:top:Popular")];
        let added = append(root, &[page, title("tt1", "A Film")]).unwrap();
        assert_eq!(added, 1, "a title already there is not added again");

        // A rails refresh replaces the rails and keeps the page.
        refresh(root, &[], || Ok(Fetched { titles: vec![title("tt3", "New Film")], answered: Vec::new() })).unwrap();
        let items = stored(root);
        assert_eq!(ids(&items), ["tt3", "tt2"]);
        assert!(items.iter().any(|title| title.key.id.as_str() == "tt2" && !title.rails.is_empty()), "the page is still marked");
        assert_eq!(items[1].release.as_deref(), Some("2026"), "the page keeps its metadata");
    }

    #[test]
    fn a_collection_rail_follows_its_page_and_keeps_what_it_could_not_ask() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_root(root, &[title("tt1", "A Film")]).unwrap();
        let mut in_rail = title("tt2", "Top Rated Film");
        in_rail.rails = vec![RailMark::collection("tmdb:top-rated:movie")];
        append(root, &[in_rail]).unwrap();

        // The rail is on and this refresh could not ask (offline): its last good
        // titles stay, which is what the tab draws while the source is away.
        let rail = ["tmdb:top-rated:movie".to_owned()];
        let nothing = || Ok(Fetched { titles: vec![title("tt1", "A Film")], answered: Vec::new() });
        refresh(root, &rail, nothing).unwrap();
        assert!(stored(root).iter().any(|title| title.key.id.as_str() == "tt2"), "an unreachable rail keeps its titles");

        // The rail answered, and its page no longer holds the title: the rail
        // follows the page rather than accumulating what it once showed.
        let answered = || Ok(Fetched { titles: vec![title("tt1", "A Film")], answered: vec!["tmdb:top-rated:movie".to_owned()] });
        refresh(root, &rail, answered).unwrap();
        assert!(!stored(root).iter().any(|title| title.key.id.as_str() == "tt2"), "a title off the page leaves the rail");

        // Turned off: the library no longer holds its titles either, whether or
        // not the source answered.
        let mut popular = title("tt4", "Another Rail Film");
        popular.rails = vec![RailMark::collection("tmdb:popular:movie")];
        append(root, &[popular]).unwrap();
        refresh(root, &[], || Ok(Fetched { titles: vec![title("tt1", "A Film")], answered: Vec::new() })).unwrap();
        assert_eq!(ids(&stored(root)), ["tt1"], "a rail the person turned off leaves no titles behind");
    }

    #[test]
    fn replacing_a_feed_drops_only_its_pages() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_root(root, &[title("tt1", "A Film")]).unwrap();
        let mut old = title("tt2", "Old Page");
        old.rails = vec![RailMark::feed("cinemeta", "movie:top:Popular")];
        let mut other = title("tt3", "Other Kind");
        other.key.kind = TitleKind::Series;
        other.rails = vec![RailMark::feed("cinemeta", "series:top:Popular")];
        append(root, &[old, other]).unwrap();

        let mut fresh = title("tt9", "New Page");
        fresh.rails = vec![RailMark::feed("cinemeta", "movie:imdbRating:Rating")];
        let added = replace(root, "movie", &[fresh]).unwrap();
        assert_eq!(added, 1);

        let ids = ids(&stored(root));
        assert!(ids.contains(&"tt1".to_owned()), "the rails stay: {ids:?}");
        assert!(ids.contains(&"tt9".to_owned()), "the new page is there: {ids:?}");
        assert!(!ids.contains(&"tt2".to_owned()), "the previous feed's page is gone: {ids:?}");
        assert!(ids.contains(&"tt3".to_owned()), "the other kind is untouched: {ids:?}");
    }

    #[test]
    fn a_merge_keeps_the_progress_of_what_is_already_there() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-12T20:00:00Z").expect("an rfc3339 instant").with_timezone(&chrono::Utc);
        let mut watched = title("tt1", "A Film");
        watched.watched = Some(heronite_watch_core::title::Watched { at: Some(at), minutes: 42.0, ..Default::default() });
        write_root(root, &[watched]).unwrap();

        let mut page = title("tt2", "Another Film");
        page.rails = vec![RailMark::feed("cinemeta", "movie:top:Popular")];
        append(root, &[page]).unwrap();
        let items = stored(root);
        let watched = items.iter().find(|title| title.key.id.as_str() == "tt1").expect("the title is still there");
        assert_eq!(watched.watched.as_ref().and_then(|seen| seen.at), Some(at));
        assert_eq!(watched.watched.as_ref().map(|seen| seen.minutes), Some(42.0));
    }

    #[test]
    fn one_title_in_two_providers_is_two_records_one_link_and_one_title() {
        let dir = tempfile::tempdir().unwrap();
        let mut film = title("tt1", "A Film");
        film.providers.insert(ProviderId::declared("youtube"));
        write_root(dir.path(), &[film]).unwrap();
        let paths = DataPaths::at(dir.path().to_path_buf());

        let library: Value = json_store::read(&paths.sideload_library()).unwrap();
        let games = library["games"].as_array().expect("the games");
        assert_eq!(games.len(), 2, "one record per provider");
        let mut runners: Vec<&str> = games.iter().filter_map(|game| game["runner"].as_str()).collect();
        runners.sort_unstable();
        assert_eq!(runners, ["stremio", "youtube"]);

        // Upstream's own "one game, several stores": the two records are linked,
        // so the grid collapses them and offers the provider to play with.
        let copies: Value = json_store::read(&paths.copies()).unwrap();
        let links = copies["links"].as_array().expect("a link list");
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].as_array().map(Vec::len), Some(2), "{copies}");

        let back = stored(dir.path());
        assert_eq!(back.len(), 1, "and the projection reads back as one title");
        assert_eq!(back[0].providers.len(), 2);
    }

    #[test]
    fn a_pair_the_person_ungrouped_is_not_linked_again() {
        let dir = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(dir.path().to_path_buf());
        let mut film = title("tt1", "A Film");
        film.providers.insert(ProviderId::declared("youtube"));
        // The person took the pair apart by hand; the writer must respect it.
        let copies =
            CopiesFile { apart: vec![["stremio_stremio:tt1".to_owned(), "youtube_youtube:tt1".to_owned()]], ..CopiesFile::default() };
        json_store::write(&paths.copies(), &copies).unwrap();

        write_root(dir.path(), &[film]).unwrap();

        let written: Value = json_store::read(&paths.copies()).unwrap();
        assert!(written["links"].as_array().is_none_or(Vec::is_empty), "{written}");
        assert_eq!(written["apart"].as_array().map(Vec::len), Some(1), "and their decision is kept");
    }

    #[test]
    fn a_placeholder_is_written_unavailable_and_comes_back_as_one() {
        let dir = tempfile::tempdir().unwrap();
        let placeholder = Title::new(TitleKey::new(TitleKind::Movie, TitleId::imdb("tt9")), "Imported Film");
        write_root(dir.path(), &[placeholder]).unwrap();

        let library: Value = json_store::read(&DataPaths::at(dir.path().to_path_buf()).sideload_library()).unwrap();
        assert_eq!(library["games"][0]["runner"], IMPORTED_ADDON);
        assert_eq!(library["games"][0]["is_installed"], false);
        assert_eq!(library["games"][0]["extra"]["imported"], true);
        assert_eq!(library["games"][0]["app_name"], "imported:tt9");

        let back = stored(dir.path());
        assert_eq!(back.len(), 1);
        assert!(!back[0].available(), "and it comes back with no provider");
    }

    #[test]
    fn the_watch_state_is_written_once_per_title() {
        let dir = tempfile::tempdir().unwrap();
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-12T20:00:00Z").expect("an rfc3339 instant").with_timezone(&chrono::Utc);
        let mut film = title("tt1", "A Film");
        film.providers.insert(ProviderId::declared("youtube"));
        film.watched = Some(heronite_watch_core::title::Watched { at: Some(at), minutes: 42.0, ..Default::default() });
        write_root(dir.path(), &[film]).unwrap();

        let timestamps: Value = json_store::read(&DataPaths::at(dir.path().to_path_buf()).timestamps()).unwrap();
        let carrying = timestamps.as_object().expect("the timestamps map").values().filter(|stamp| stamp["lastPlayed"].is_string()).count();
        assert_eq!(carrying, 1, "one copy carries the watch state, so the copies total does not double");
        assert_eq!(timestamps["stremio:tt1"]["totalPlayed"], 42.0);
    }

    #[test]
    fn a_film_and_a_series_sharing_an_id_collide_on_the_wire_and_the_first_wins() {
        let dir = tempfile::tempdir().unwrap();
        let film = title("tt1", "A Film");
        let mut series = title("tt1", "A Series");
        series.key.kind = TitleKind::Series;
        write_root(dir.path(), &[film, series]).unwrap();

        let library: Value = json_store::read(&DataPaths::at(dir.path().to_path_buf()).sideload_library()).unwrap();
        let games = library["games"].as_array().expect("the games");
        assert_eq!(games.len(), 1, "the key carries no kind, so the second would overwrite the first: {} games", games.len());
        assert_eq!(games[0]["extra"]["stremioType"], "movie", "the first title wins");
    }

    #[test]
    fn the_projection_keys_carry_the_provider() {
        let dir = tempfile::tempdir().unwrap();
        write_root(dir.path(), &[title("tt1", "A Film")]).unwrap();
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

    #[test]
    fn the_stamp_is_written_in_the_shape_upstream_sorts_by() {
        let dir = tempfile::tempdir().unwrap();
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-12T20:00:00Z").expect("an rfc3339 instant").with_timezone(&chrono::Utc);
        let mut watched = title("tt1", "A Film");
        watched.watched = Some(heronite_watch_core::title::Watched { at: Some(at), minutes: 42.5, ..Default::default() });
        write_root(dir.path(), &[watched]).unwrap();

        let timestamps: Value = json_store::read(&DataPaths::at(dir.path().to_path_buf()).timestamps()).unwrap();
        assert_eq!(timestamps["stremio:tt1"]["lastPlayed"], "2026-11-12T20:00:00.000Z");
        assert_eq!(timestamps["stremio:tt1"]["totalPlayed"], 42.5);
    }

    #[test]
    fn a_started_film_keeps_its_position_and_what_a_catalogue_said_about_it() {
        let dir = tempfile::tempdir().unwrap();
        let at = chrono::DateTime::parse_from_rfc3339("2026-11-17T19:49:06Z").expect("an rfc3339 instant").with_timezone(&chrono::Utc);
        let mut film = title("tt1", "A Film");
        film.released = Some("2026-11-14".into());
        film.runtime = Some("173 min".into());
        // Stremio counts no minute for a film that was only started: the
        // position is what says it was, and the screens read it off the record.
        film.watched = Some(heronite_watch_core::title::Watched {
            at: Some(at),
            minutes: 0.0,
            position_ms: 2_271_644,
            duration_ms: 6_628_768,
            finished: false,
        });
        write_root(dir.path(), &[film]).unwrap();

        let paths = DataPaths::at(dir.path().to_path_buf());
        let library: Value = json_store::read(&paths.sideload_library()).unwrap();
        let extra = &library["games"][0]["extra"];
        assert_eq!(extra[POSITION_KEY], 2_271_644);
        assert_eq!(extra[DURATION_KEY], 6_628_768);
        assert_eq!(extra[FINISHED_KEY], false);
        assert_eq!(extra[RELEASED_KEY], "2026-11-14");
        assert_eq!(extra[RUNTIME_KEY], "173 min");

        let back = stored(dir.path());
        let seen = back[0].watched.as_ref().expect("the started record came back");
        assert_eq!(
            seen.state(),
            heronite_watch_core::progress::WatchedState::InProgress { position_ms: 2_271_644, duration_ms: 6_628_768 }
        );
        assert_eq!(back[0].released.as_deref(), Some("2026-11-14"));
        assert_eq!(back[0].runtime.as_deref(), Some("173 min"));
    }
}

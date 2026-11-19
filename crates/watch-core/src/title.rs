//! What Watch calls a title: a film or a series, and the stream provider that can
//! play it.
//!
//! This is the rework's centre. Watch used to borrow the collection's idea of a
//! *game from a store* — `runner`, `app_name`, `installed` — with its own data
//! smuggled through `extra`. The structure stays (library, rails, tiles,
//! progress); the words and the model become ours: **stream providers** and
//! **movies/series**.

use serde::{Deserialize, Serialize};

use crate::types::MovieItem;

/// The `addon` marker an imported placeholder carries: a title a provider knows
/// and no stream source here has.
pub const IMPORTED_ADDON: &str = "imported";

/// A film or a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleKind {
    Movie,
    Series,
}

impl TitleKind {
    /// The kind a media source's own word means ("movie", "series", "show", …).
    pub fn of(raw: &str) -> Self {
        match raw {
            "series" | "show" | "tv" => Self::Series,
            _ => Self::Movie,
        }
    }

    /// The word the library stores it by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }
}

/// The id a title is known by here: its IMDb id when the source has one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TitleId(pub String);

/// A stream provider: the thing that can play a title (Stremio today, others
/// as they come). It is **not** a store, and it owns no library.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderId(pub String);

/// What the screens draw for a title.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Art {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
}

/// One title in the Watch library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Title {
    pub id: TitleId,
    pub kind: TitleKind,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<String>,
    #[serde(default)]
    pub art: Art,
    /// The stream provider that can play it; `None` for an imported placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
    /// Whether a stream source here can play it at all. A placeholder cannot —
    /// that is what "not currently present in any available stream source" means,
    /// and what the tiles grey.
    pub available: bool,
    /// The rails and catalogs that brought it, so a screen can mark them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rails: Vec<String>,
}

impl Title {
    /// The title a library item is, as Watch sees it.
    ///
    /// An item marked [`IMPORTED_ADDON`] is a placeholder: it is listed and it has
    /// no provider, because nothing here can play it.
    pub fn of_item(item: &MovieItem) -> Self {
        let imported = item.addon == IMPORTED_ADDON;
        let kind = TitleKind::of(&item.kind);
        let provider = (item.provider.trim().is_empty() && !imported).then(|| ProviderId("stremio".to_owned()));
        let provider = match item.provider.trim() {
            "" if imported => None,
            "" => provider,
            id => Some(ProviderId(id.to_owned())),
        };
        Self {
            id: TitleId(item.id.clone()),
            kind,
            name: item.name.clone(),
            year: item.year.clone(),
            art: Art { poster: item.poster.clone(), background: item.background.clone(), logo: item.logo.clone() },
            available: provider.is_some(),
            provider,
            rails: item.catalog.clone().into_iter().collect(),
        }
    }

    /// Whether `provider` is the one that plays this title.
    pub fn played_by(&self, provider: &str) -> bool {
        self.provider.as_ref().is_some_and(|own| own.0 == provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, addon: &str, provider: &str) -> MovieItem {
        MovieItem {
            id: id.into(),
            kind: "movie".into(),
            name: "A Film".into(),
            provider: provider.into(),
            addon: addon.into(),
            ..MovieItem::default()
        }
    }

    #[test]
    fn a_library_item_is_a_title_with_its_stream_provider() {
        let title = Title::of_item(&item("tt1", "cinemeta", "stremio"));
        assert_eq!(title.kind, TitleKind::Movie);
        assert!(title.available);
        assert!(title.played_by("stremio"));
        assert_eq!(title.id.0, "tt1");
    }

    #[test]
    fn an_imported_placeholder_has_no_provider_and_is_not_available() {
        let title = Title::of_item(&item("tt9", IMPORTED_ADDON, ""));
        assert!(!title.available);
        assert!(title.provider.is_none());
        assert!(!title.played_by("stremio"), "nothing here plays it");
        assert_eq!(title.kind, TitleKind::Movie);
    }

    #[test]
    fn a_series_is_a_series_and_a_show_counts_as_one() {
        assert_eq!(TitleKind::of("series"), TitleKind::Series);
        assert_eq!(TitleKind::of("show"), TitleKind::Series);
        assert_eq!(TitleKind::of("movie"), TitleKind::Movie);
        assert_eq!(TitleKind::Series.as_str(), "series");
    }
}

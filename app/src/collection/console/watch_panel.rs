//! The Watch Filters panel: Type, Sort, Genre and Provider, with search on
//! top. This is fork-owned; the upstream console only dispatches here.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::{tr, trc};
use heronite_watch::catalog::CatalogSort;

use super::filters::{segments, Item};
use crate::collection::CollectionView;
use crate::console::kit::{self, u};
use crate::console::motion;
use crate::console::pointer::{self, Clickable};
use crate::console::Pad;
use crate::fork::watch::sources as watch_sources;
use crate::icons::Icon;
use crate::theme;
use crate::ui::icon;

thread_local! {
    /// The panel's scroll, kept between renders: a handle made per frame cannot
    /// remember where it was, so the list would jump back to the top.
    static FILTER_SCROLL: ScrollHandle = ScrollHandle::new();
    /// The item the panel's focus was on last frame. The list scrolls to the
    /// focused row when the pad moves it, and only then — a wheel scroll must
    /// not be undone by the next frame.
    static FILTER_FOCUS: std::cell::Cell<Option<Item>> = const { std::cell::Cell::new(None) };
}

/// The panel is opening: the first frame's focus is what the list scrolls to,
/// even when it is the search field it last rested on.
pub(super) fn filter_panel_opened() {
    FILTER_FOCUS.with(|last| last.set(None));
}

impl CollectionView {
    pub(super) fn render_watch_filters(&mut self) -> Option<AnyElement> {
        let focus = self.focused_filter()?;
        self.lib.as_ref()?;
        let (target, spec) = self.filters_target();
        let games = self.console_games(&spec.source);
        let genres = watch_sources::genres(&games);
        let providers = watch_sources::providers(&games);
        let shown = self.grid_keys(target, &spec).len();
        let f = self.tab_nav(target).grid.filters.clone();
        let search = self.console.search.clone();
        // A search is a search: TMDB's `/search` takes no discover filters, so
        // the browse controls are hidden while one is typed instead of being
        // shown and ignored.
        let searching = !search.trim().is_empty();
        let fixed = spec.source.starts_with("type:");

        let tint = |on: bool| -> Hsla {
            if on {
                theme::console_on_focus().into()
            } else {
                theme::text_muted().into()
            }
        };
        let items = self.filter_items();
        let at = items.iter().position(|i| *i == focus).unwrap_or(0);
        let to = |item: Item, then: &[Pad]| -> Vec<Pad> {
            let Some(j) = items.iter().position(|i| *i == item) else { return Vec::new() };
            let mut pads = pointer::along(at, j, Pad::Up, Pad::Down);
            pads.extend_from_slice(then);
            pads
        };
        let field =
            kit::row_base(focus == Item::Search)
                .on_hover_pads(to(Item::Search, &[]))
                .on_pads(to(Item::Search, &[Pad::A]))
                .when(focus != Item::Search, |d| d.bg(kit::ink(0.07)))
                .child(div().relative().child(icon(Icon::FaSearch, u(18.), tint(focus == Item::Search))))
                .child(
                    div().relative().flex_1().min_w_0().truncate().text_size(u(17.)).when(search.is_empty(), |d| d.opacity(0.6)).child(
                        if search.is_empty() { SharedString::from(tr!("Search titles")) } else { format!("{search}\u{2502}").into() },
                    ),
                );
        let kinds: Vec<(SharedString, Option<usize>)> =
            [(tr!("All"), 0usize), (tr!("Movies"), 1), (tr!("Series"), 2)].into_iter().map(|(label, _)| (label.into(), None)).collect();
        let sorts: Vec<(SharedString, Option<usize>)> =
            [(tr!("Popular"), 0usize), (tr!("Title"), 1), (tr!("Year"), 2), (tr!("Recently watched"), 3)]
                .into_iter()
                .map(|(label, _)| (label.into(), None))
                .collect();
        let genre = f.genre.checked_sub(1).and_then(|i| genres.get(i)).cloned().unwrap_or_else(|| tr!("Any genre").to_string());
        let provider = f.provider.checked_sub(1).and_then(|i| providers.get(i)).cloned().unwrap_or_else(|| tr!("Any provider").to_string());
        let row = |item: Item, label: String, ic: Icon, value: AnyElement| {
            let on = focus == item;
            kit::row_base(on)
                .on_hover_pads(to(item, &[]))
                .on_pads(to(item, &[Pad::A]))
                .child(div().relative().child(icon(ic, u(20.), tint(on))))
                .child(div().relative().flex_1().min_w_0().truncate().text_size(u(17.)).child(label))
                .child(div().relative().child(value))
        };
        let scroll = FILTER_SCROLL.with(Clone::clone);
        // The panel's rows in the order it draws them, each with the item it
        // focuses (a caption focuses nothing). A row's place in this list is its
        // child index, which is what the list scrolls to.
        let mut rows: Vec<(Option<Item>, AnyElement)> = vec![(Some(Item::Search), field.into_any_element())];
        if !fixed {
            rows.push((None, kit::caption(tr!("Media type")).into_any_element()));
            rows.push((
                Some(Item::Kind),
                segments(&kinds, f.kind, focus == Item::Kind, |k| to(Item::Kind, &pointer::along(f.kind, k, Pad::Left, Pad::Right)))
                    .into_any_element(),
            ));
        }
        // A catalog tab is browsed by what the provider declares: its catalogs,
        // the orders it can page by (hidden when there is only one) and its own
        // filters. A search carries none of them, so it keeps the field alone.
        let catalog_kind = crate::collection::watch_catalog::feed_kind(&spec.source);
        if let Some(kind) = catalog_kind.filter(|_| !searching) {
            let catalogs = self.watch_catalogs_of(kind);
            let source = self.watch_feed_source(kind);
            let chosen = self.watch_feed_catalog(kind);
            let at = catalogs.iter().position(|(id, catalog)| *id == source && catalog.id == chosen).unwrap_or(0);
            // A chip names its source only when more than one source is on, so a
            // single-source setup reads as it always did.
            let several = {
                let mut sources: Vec<&str> = catalogs.iter().map(|(id, _)| id.as_str()).collect();
                sources.sort_unstable();
                sources.dedup();
                sources.len() > 1
            };
            let names: Vec<SharedString> = catalogs
                .iter()
                .map(|(id, catalog)| {
                    let name = catalog_name(&catalog.id, &catalog.name);
                    let source = self.watch_source_name(id);
                    let label = if several && source != name { format!("{source} · {name}") } else { name.to_string() };
                    SharedString::from(label)
                })
                .collect();
            let labels: Vec<(SharedString, Option<usize>)> = names.into_iter().map(|name| (name, None)).collect();
            rows.push((None, kit::caption(tr!("Catalog")).into_any_element()));
            rows.push((
                Some(Item::Catalog),
                segments(&labels, at, focus == Item::Catalog, |k| to(Item::Catalog, &pointer::along(at, k, Pad::Left, Pad::Right)))
                    .into_any_element(),
            ));
            if let Some((_, descriptor)) = self.watch_descriptor(kind) {
                if descriptor.sorts.len() > 1 {
                    let order = self.watch_feed_sort(kind);
                    let at = descriptor.sorts.iter().position(|sort| *sort == order).unwrap_or(0);
                    let options: Vec<(SharedString, Option<usize>)> =
                        descriptor.sorts.iter().map(|sort| (sort_name(*sort), None)).collect();
                    rows.push((None, kit::caption(tr!("Sort")).into_any_element()));
                    rows.push((
                        Some(Item::Sort),
                        segments(&options, at, focus == Item::Sort, |k| to(Item::Sort, &pointer::along(at, k, Pad::Left, Pad::Right)))
                            .into_any_element(),
                    ));
                }
                for (index, filter) in descriptor.filters.iter().enumerate() {
                    let chosen_value = self.watch_feed_filters(kind).into_iter().find(|(id, _)| *id == filter.id).map(|(_, value)| value);
                    let at = chosen_value.as_ref().and_then(|value| filter.values.iter().position(|v| v == value)).map_or(0, |i| i + 1);
                    let text = chosen_value.map(|value| filter_value(&filter.id, &value)).unwrap_or_else(|| tr!("All").into());
                    let on = focus == Item::Filter(index);
                    rows.push((
                        Some(Item::Filter(index)),
                        row(
                            Item::Filter(index),
                            filter_label(&filter.id, &filter.label).to_string(),
                            Icon::GridView,
                            kit::choice_clickable(
                                text,
                                on,
                                at > 0,
                                at < filter.values.len(),
                                140.,
                                to(Item::Filter(index), &[Pad::Left]),
                                to(Item::Filter(index), &[Pad::Right]),
                            )
                            .into_any_element(),
                        )
                        .into_any_element(),
                    ));
                }
            }
        } else if catalog_kind.is_none() {
            // Not a catalog tab: the local filters and sorts are the ones to show.
            rows.push((None, kit::caption(tr!("Sort")).into_any_element()));
            rows.push((
                Some(Item::Sort),
                segments(&sorts, f.sort, focus == Item::Sort, |k| to(Item::Sort, &pointer::along(f.sort, k, Pad::Left, Pad::Right)))
                    .into_any_element(),
            ));
            if !genres.is_empty() {
                let on = focus == Item::Genre;
                rows.push((
                    Some(Item::Genre),
                    row(
                        Item::Genre,
                        tr!("Genre").to_string(),
                        Icon::GridView,
                        kit::choice_clickable(
                            genre,
                            on,
                            f.genre > 0,
                            f.genre < genres.len(),
                            140.,
                            to(Item::Genre, &[Pad::Left]),
                            to(Item::Genre, &[Pad::Right]),
                        )
                        .into_any_element(),
                    )
                    .into_any_element(),
                ));
            }
            if !providers.is_empty() {
                let on = focus == Item::Provider;
                rows.push((
                    Some(Item::Provider),
                    row(
                        Item::Provider,
                        tr!("Provider").to_string(),
                        Icon::Storefront,
                        kit::choice_clickable(
                            provider,
                            on,
                            f.provider > 0,
                            f.provider < providers.len(),
                            140.,
                            to(Item::Provider, &[Pad::Left]),
                            to(Item::Provider, &[Pad::Right]),
                        )
                        .into_any_element(),
                    )
                    .into_any_element(),
                ));
            }
        }
        // Keep the focused row in sight when the pad moves. Only then: a wheel
        // scroll is not undone by the next frame's render.
        if FILTER_FOCUS.with(|last| last.replace(Some(focus)) != Some(focus)) {
            if let Some(index) = rows.iter().position(|(item, _)| *item == Some(focus)) {
                scroll.scroll_to_item(index);
            }
        }
        let body = div()
            .id("console-watch-filters-list")
            .flex()
            .flex_col()
            .gap(u(6.))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .children(rows.into_iter().map(|(_, row)| row));
        let footer = div()
            .flex()
            .items_center()
            .px(u(8.))
            .pt(u(12.))
            .border_t_1()
            .border_color(kit::ink(0.08))
            .text_size(u(15.))
            .text_color(theme::text_muted())
            .child(tr!("{shown} of {total} titles", shown, total = games.len()));
        let panel = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(u(470.))
            .flex()
            .flex_col()
            .gap(u(14.))
            .px(u(18.))
            .pt(u(26.))
            .pb(u(70.))
            .bg(theme::console_sidebar())
            .border_l_1()
            .border_color(kit::ink(0.08))
            .shadow(kit::shadow(0.5, 0., 40.))
            .eats_clicks()
            .child(div().px(u(8.)).child(kit::title(tr!("Filters"), 30.)))
            .child(body)
            .child(footer);
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_pads([Pad::B])
                .child(motion::enter("console-watch-filters", panel, 280, 40., 0.))
                .into_any_element(),
        )
    }
}

/// A catalog's chip: the ids we know read in the person's language, anything
/// else as the provider named it.
fn catalog_name(id: &str, name: &str) -> SharedString {
    match id {
        "top" => tr!("Popular").to_string().into(),
        "imdbRating" => tr!("Top rated").to_string().into(),
        "year" => tr!("Year").to_string().into(),
        _ => SharedString::from(name.to_owned()),
    }
}

/// A sort's label; a provider picks from the closed set, so these are known.
fn sort_name(sort: CatalogSort) -> SharedString {
    match sort {
        CatalogSort::Popular => tr!("Popular").to_string().into(),
        CatalogSort::Rating => tr!("Top rated").to_string().into(),
        CatalogSort::Newest => tr!("Year").to_string().into(),
        CatalogSort::Title => tr!("Title").to_string().into(),
    }
}

/// A filter's label, localized for the ids we know and the provider's own
/// wording otherwise.
fn filter_label(id: &str, label: &str) -> SharedString {
    match id {
        "genre" => tr!("Genre").to_string().into(),
        "year" => tr!("Year").to_string().into(),
        "rating" => tr!("Rating").to_string().into(),
        "length" => tr!("Length").to_string().into(),
        "language" => tr!("Original language").to_string().into(),
        "adult" => tr!("Adult content").to_string().into(),
        "status" => tr!("Status").to_string().into(),
        _ => SharedString::from(label.to_owned()),
    }
}

/// A filter's value as the panel says it. Length, adult and status are stored as
/// stable ids (the source maps them to TMDB's numbers), so their words are ours;
/// anything else is the source's own value, as genres always were.
fn filter_value(id: &str, value: &str) -> SharedString {
    match (id, value) {
        ("length", "short") => tr!("Under 90 min").into(),
        ("length", "medium") => tr!("90–120 min").into(),
        ("length", "long") => tr!("Over 120 min").into(),
        ("adult", "hide") => tr!("Hide").into(),
        ("adult", "show") => tr!("Show").into(),
        ("status", "returning") => tr!("Returning").into(),
        ("status", "ended") => tr!("Ended").into(),
        // A show's status, not a download's: the plain word means "aborted"
        // elsewhere in the app.
        ("status", "canceled") => trc!("series", "Canceled").into(),
        _ => SharedString::from(value.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::{filter_label, filter_value};

    use core::prelude::v1::test;

    #[test]
    fn the_panels_fields_are_named_in_the_persons_language() {
        assert_eq!(filter_label("rating", "rating").as_ref(), "Rating");
        assert_eq!(filter_label("length", "length").as_ref(), "Length");
        assert_eq!(filter_label("language", "language").as_ref(), "Original language");
        assert_eq!(filter_label("adult", "adult").as_ref(), "Adult content");
        assert_eq!(filter_label("status", "status").as_ref(), "Status");
        assert_eq!(filter_label("anything", "Anything").as_ref(), "Anything", "a source's unknown field keeps its own name");
    }

    #[test]
    fn a_field_value_is_read_in_words_or_kept_as_the_source_said_it() {
        assert_eq!(filter_value("length", "short").as_ref(), "Under 90 min");
        assert_eq!(filter_value("length", "medium").as_ref(), "90–120 min");
        assert_eq!(filter_value("length", "long").as_ref(), "Over 120 min");
        assert_eq!(filter_value("adult", "show").as_ref(), "Show");
        assert_eq!(filter_value("status", "canceled").as_ref(), "Canceled");
        assert_eq!(filter_value("genre", "Science Fiction").as_ref(), "Science Fiction", "TMDB's own genre names");
        assert_eq!(filter_value("language", "日本語").as_ref(), "日本語", "a language reads in its own words");
    }
}

//! The Watch Filters panel: Type, Sort, Genre and Provider, with search on
//! top. This is fork-owned; the upstream console only dispatches here.

use gpui_kit::prelude::*;
use gpui_kit::*;
use heronite_i18n::tr;
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
        let mut body = div().flex().flex_col().gap(u(6.)).child(field);
        if !fixed {
            body = body.child(kit::caption(tr!("Media type"))).child(segments(&kinds, f.kind, focus == Item::Kind, |k| {
                to(Item::Kind, &pointer::along(f.kind, k, Pad::Left, Pad::Right))
            }));
        }
        // A catalog tab is browsed by what the provider declares: its catalogs,
        // the orders it can page by (hidden when there is only one) and its own
        // filters.
        let catalog_kind = crate::collection::watch_catalog::feed_kind(&spec.source);
        if let Some(kind) = catalog_kind {
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
            body = body
                .child(kit::caption(tr!("Catalog")))
                .child(segments(&labels, at, focus == Item::Catalog, |k| to(Item::Catalog, &pointer::along(at, k, Pad::Left, Pad::Right))));
            if let Some((_, descriptor)) = self.watch_descriptor(kind) {
                if descriptor.sorts.len() > 1 {
                    let order = self.watch_feed_sort(kind);
                    let at = descriptor.sorts.iter().position(|sort| *sort == order).unwrap_or(0);
                    let options: Vec<(SharedString, Option<usize>)> =
                        descriptor.sorts.iter().map(|sort| (sort_name(*sort), None)).collect();
                    body = body.child(kit::caption(tr!("Sort"))).child(segments(&options, at, focus == Item::Sort, |k| {
                        to(Item::Sort, &pointer::along(at, k, Pad::Left, Pad::Right))
                    }));
                }
                for (index, filter) in descriptor.filters.iter().enumerate() {
                    let chosen_value = self.watch_feed_filters(kind).into_iter().find(|(id, _)| *id == filter.id).map(|(_, value)| value);
                    let at = chosen_value.as_ref().and_then(|value| filter.values.iter().position(|v| v == value)).map_or(0, |i| i + 1);
                    let text = chosen_value.unwrap_or_else(|| tr!("All").to_string());
                    let on = focus == Item::Filter(index);
                    body = body.child(row(
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
                    ));
                }
            }
        }
        // Not a catalog tab: the local filters and sorts are the ones to show.
        if catalog_kind.is_none() {
            body = body.child(kit::caption(tr!("Sort"))).child(segments(&sorts, f.sort, focus == Item::Sort, |k| {
                to(Item::Sort, &pointer::along(f.sort, k, Pad::Left, Pad::Right))
            }));
        }
        if catalog_kind.is_some() {
            // The rows above already are this tab's browse controls.
        }
        if catalog_kind.is_none() && !genres.is_empty() {
            let on = focus == Item::Genre;
            body = body.child(row(
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
            ));
        }
        if catalog_kind.is_none() && !providers.is_empty() {
            let on = focus == Item::Provider;
            body = body.child(row(
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
            ));
        }
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
            .child(div().flex_1())
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
        _ => SharedString::from(label.to_owned()),
    }
}

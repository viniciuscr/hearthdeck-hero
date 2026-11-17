//! The Shelf tab's provider rails, as the app names them.
//!
//! A source declares a rail's name in its own English (`Films · Top rated`);
//! the header is drawn in the person's language instead. A rule says what it is
//! — the kind it answers and the order it asks — so that is what names it, and a
//! rule that is more than that (a filter, a search) keeps the source's words.

use heronite_i18n::tr;
use heronite_watch::catalog::CatalogSort;
use heronite_watch::collections::CollectionDescriptor;

/// What the rail's header says.
pub fn title(descriptor: &CollectionDescriptor) -> String {
    let kind = match descriptor.kind.as_str() {
        "movie" => tr!("Movies"),
        "series" => tr!("Series"),
        _ => return descriptor.name.clone(),
    };
    if descriptor.rule.search.is_some() || !descriptor.rule.filters.is_empty() {
        return descriptor.name.clone();
    }
    let order = match descriptor.rule.sort {
        CatalogSort::Popular => tr!("Popular"),
        CatalogSort::Rating => tr!("Top rated"),
        CatalogSort::Newest => tr!("Newest"),
        CatalogSort::Title => tr!("Title"),
    };
    format!("{kind} · {order}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use heronite_watch::catalog::CatalogRequest;

    fn rail(name: &str, kind: &str, sort: CatalogSort) -> CollectionDescriptor {
        CollectionDescriptor {
            id: format!("test:{kind}"),
            name: name.to_owned(),
            kind: kind.to_owned(),
            rule: CatalogRequest { sort, ..CatalogRequest::first(kind) },
        }
    }

    #[test]
    fn a_rule_is_named_after_the_kind_and_the_order() {
        let named = |kind: &str, order: &str| format!("{kind} · {order}");
        assert_eq!(title(&rail("Films · Top rated", "movie", CatalogSort::Rating)), named(tr!("Movies"), tr!("Top rated")));
        assert_eq!(title(&rail("Series · Popular", "series", CatalogSort::Popular)), named(tr!("Series"), tr!("Popular")));
        assert_eq!(title(&rail("Films · Newest", "movie", CatalogSort::Newest)), named(tr!("Movies"), tr!("Newest")));
    }

    #[test]
    fn a_rule_the_app_cannot_name_keeps_the_sources_words() {
        let mut filtered = rail("Films · Sci-Fi", "movie", CatalogSort::Popular);
        filtered.rule.filters = vec![("genre".to_owned(), "Science Fiction".to_owned())];
        assert_eq!(title(&filtered), "Films · Sci-Fi", "a filter is the source's to name");

        let mut searched = rail("Films · In cinemas", "movie", CatalogSort::Newest);
        searched.rule.search = Some("matrix".to_owned());
        assert_eq!(title(&searched), "Films · In cinemas");

        assert_eq!(title(&rail("Books · Top rated", "book", CatalogSort::Rating)), "Books · Top rated");
    }
}

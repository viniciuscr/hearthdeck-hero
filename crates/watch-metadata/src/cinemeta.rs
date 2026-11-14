//! Cinemeta, Stremio's metadata addon, as a Watch metadata source.
//!
//! Cinemeta is an addon like any other: the source only knows its manifest
//! URL and speaks the `meta` resource over it. It is not part of the Stremio
//! media provider, and it needs no account: an unlinked install still gets a
//! film's cast and rating.

use anyhow::Result;
use heronite_watch_addon::addon::AddonClient;
use heronite_watch_core::metadata::{MetaCtx, MetaDescriptor, TitleDetails, TitleRef, WatchMetadataSource};

/// Where Cinemeta lives. Same host the Stremio provider browses its catalogs
/// on; the source keeps its own copy so it depends on no provider crate.
pub const MANIFEST: &str = "https://v3-cinemeta.strem.io/manifest.json";

const DESCRIPTOR: MetaDescriptor = MetaDescriptor { id: "cinemeta", name: "Cinemeta", applies_to: &[] };

/// A source over one addon's `meta` resource.
pub struct Cinemeta {
    manifest: &'static str,
}

impl Cinemeta {
    /// The addon at `manifest`, for a test or a second deployment.
    pub const fn at(manifest: &'static str) -> Self {
        Self { manifest }
    }

    /// The public Cinemeta.
    pub const fn builtin() -> Self {
        Self::at(MANIFEST)
    }
}

impl WatchMetadataSource for Cinemeta {
    fn descriptor(&self) -> &MetaDescriptor {
        &DESCRIPTOR
    }

    fn details(&self, ctx: &MetaCtx<'_>, title: &TitleRef) -> Result<Option<TitleDetails>> {
        AddonClient::new(ctx.http, self.manifest)?.details(&title.kind, &title.id)
    }
}

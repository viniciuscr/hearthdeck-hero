//! Custom GitHub Proton sources added by the user in the Wine Manager.
//!
//! Stored in `store/wine-sources.json`. Releases are parsed with the same
//! GitHub pipeline as built-in sources, using an automatic archive picker
//! (with an optional variant filter chosen when adding the source).

use std::collections::HashSet;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use heronite_data::{json_store, DataPaths};
use heronite_i18n::tr;
use serde::{Deserialize, Serialize};

use crate::checks::{self, Remote};
use crate::github::ApiError;
use crate::install::is_safe_name;
use crate::repos::{self, GhAsset, GhRelease, Repository, Source};
use crate::store;

const KEY: &str = "sources";
const PROBE_RELEASES: usize = 20;
const CUSTOM_ARCHIVES: &[&str] = &[".tar.xz", ".tar.gz", ".tar.zst"];

/// A user-added GitHub repository that publishes Proton `.tar.*` releases.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSource {
    /// Display name and `type` stored in `wine-downloader-info.json`.
    pub kind: String,
    /// Normalized `owner/name` on GitHub.
    pub github: String,
    /// Optional substring used to select a specific archive variant (e.g.
    /// `-x86_64_v3.tar`, `-async.tar`). `None` picks the default archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_filter: Option<String>,
}

/// One selectable archive variant discovered during [`probe_github_source`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeVariant {
    /// Short label for the UI (`Default`, `x86_64_v3`, `wow64`, ...).
    pub label: String,
    /// `None` for the automatic default, or a filter token stored in [`CustomSource::asset_filter`].
    pub filter: Option<String>,
    /// Filename matched on the newest release.
    pub example_file: String,
}

/// Result of inspecting a GitHub repository before adding it as a source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeResult {
    pub github: String,
    pub suggested_name: String,
    pub newest_tag: String,
    pub release_count: usize,
    pub variants: Vec<ProbeVariant>,
    /// Raw GitHub releases JSON so adding the source seeds the cache immediately.
    pub raw_releases: String,
}

/// Parses a GitHub repository URL or `owner/repo` shorthand into normalized `owner/repo`.
pub fn parse_github_repo(input: &str) -> Result<String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err(anyhow!(tr!("Enter a GitHub repository URL or owner/repo.")));
    }
    let without_scheme = if let Some(rest) = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("http://")) {
        let rest = rest.strip_prefix("www.").unwrap_or(rest);
        let Some(after_host) = rest.strip_prefix("github.com/") else {
            return Err(anyhow!(tr!("Only GitHub repositories are supported.")));
        };
        after_host
    } else if raw.contains("://") {
        return Err(anyhow!(tr!("Only GitHub repositories are supported.")));
    } else {
        let rest = raw.strip_prefix("www.").unwrap_or(raw);
        if let Some(after_host) = rest.strip_prefix("github.com/") {
            after_host
        } else {
            raw
        }
    };

    let cleaned = without_scheme.split(['?', '#']).next().unwrap_or(without_scheme).trim_matches('/');
    let mut parts = cleaned.split('/').filter(|s| !s.is_empty());
    let Some(owner) = parts.next() else {
        return Err(anyhow!(tr!("Enter a GitHub repository as owner/repo or a github.com URL.")));
    };
    let Some(repo_raw) = parts.next() else {
        return Err(anyhow!(tr!("Enter a GitHub repository as owner/repo or a github.com URL.")));
    };
    let repo = repo_raw.strip_suffix(".git").unwrap_or(repo_raw);

    if !valid_gh_segment(owner) || !valid_gh_segment(repo) {
        return Err(anyhow!(tr!("Invalid GitHub repository name.")));
    }
    // Reject obvious non-GitHub hosts passed without scheme, like `gitlab.com/a/b`.
    if owner.contains('.') && parts.next().is_some() {
        return Err(anyhow!(tr!("Only GitHub repositories are supported.")));
    }
    Ok(format!("{owner}/{repo}"))
}

fn valid_gh_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && !s.contains("..")
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Reads saved custom sources (empty when the file is missing or unreadable).
pub fn read_custom_sources(data_paths: &DataPaths) -> Vec<CustomSource> {
    read_from(&data_paths.wine_sources())
}

fn read_from(path: &Path) -> Vec<CustomSource> {
    let doc = json_store::read_value(path).unwrap_or(serde_json::Value::Null);
    doc.get(KEY)
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(|v| serde_json::from_value::<CustomSource>(v.clone()).ok()).collect())
        .unwrap_or_default()
}

fn write_custom_sources(data_paths: &DataPaths, list: &[CustomSource]) -> Result<()> {
    let value = serde_json::to_value(list)?;
    json_store::update_value(&data_paths.wine_sources(), |doc| {
        doc[KEY] = value;
        Ok(())
    })
}

/// Returns the existing source name if `github` is already configured (built-in or custom).
pub fn existing_by_github(data_paths: &DataPaths, github: &str) -> Option<String> {
    if let Some(repo) = Repository::all_known().iter().find(|r| r.github().eq_ignore_ascii_case(github)) {
        return Some(repo.kind().to_owned());
    }
    read_custom_sources(data_paths).into_iter().find(|c| c.github.eq_ignore_ascii_case(github)).map(|c| c.kind)
}

/// Suggests a unique, filesystem-safe display name from `owner/repo`.
pub fn suggest_name(data_paths: &DataPaths, github: &str) -> String {
    let (owner, repo) = github.split_once('/').unwrap_or(("", github));
    let mut used: HashSet<String> = Repository::all_known().iter().map(|r| r.kind().to_ascii_lowercase()).collect();
    for c in read_custom_sources(data_paths) {
        used.insert(c.kind.to_ascii_lowercase());
    }
    let clean = |s: &str| {
        let t = s.trim();
        if is_safe_name(t) {
            t.to_owned()
        } else {
            t.replace(['/', '\0'], "-")
        }
    };
    let primary = clean(repo);
    if !primary.is_empty() && !used.contains(&primary.to_ascii_lowercase()) {
        return primary;
    }
    let with_owner = clean(&format!("{owner}-{repo}"));
    if !with_owner.is_empty() && !used.contains(&with_owner.to_ascii_lowercase()) {
        return with_owner;
    }
    for n in 2..=99 {
        let candidate = format!("{primary}-{n}");
        if !used.contains(&candidate.to_ascii_lowercase()) {
            return candidate;
        }
    }
    format!("{owner}-{repo}")
}

/// Validates and persists a new custom source, optionally seeding the release
/// cache from `raw_releases` returned by [`probe_github_source`].
pub fn add_custom_source(data_paths: &DataPaths, mut source: CustomSource, raw_releases: Option<&str>) -> Result<CustomSource> {
    source.kind = source.kind.trim().to_owned();
    source.github = parse_github_repo(&source.github)?;
    source.asset_filter = source.asset_filter.map(|f| f.trim().to_owned()).filter(|f| !f.is_empty());

    if !is_safe_name(&source.kind) {
        return Err(anyhow!(tr!("Enter a valid source name (without / or ..).")));
    }
    if source.kind.starts_with("Wine-") || source.kind == "Wine" {
        return Err(anyhow!(tr!("Proton source names cannot start with \"Wine-\".")));
    }
    if let Some(existing) = existing_by_github(data_paths, &source.github) {
        return Err(anyhow!(tr!("This repository is already added as {name}.", name = existing)));
    }
    let mut list = read_custom_sources(data_paths);
    let dup_kind = Repository::all_known().iter().any(|r| r.kind().eq_ignore_ascii_case(&source.kind))
        || list.iter().any(|c| c.kind.eq_ignore_ascii_case(&source.kind));
    if dup_kind {
        return Err(anyhow!(tr!("A source named {name} already exists.", name = source.kind)));
    }

    list.push(source.clone());
    write_custom_sources(data_paths, &list)?;

    if let Some(json) = raw_releases {
        let src = Source::Custom(source.clone());
        if let Ok(fetched) = repos::parse_releases_for_source(&src, json, cfg!(target_arch = "aarch64")) {
            let _ = store::apply_fetched(data_paths, fetched);
        }
    }
    Ok(source)
}

/// Removes a saved custom source and prunes its non-installed cached releases
/// (builds already installed on disk remain intact).
pub fn remove_custom_source(data_paths: &DataPaths, kind: &str) -> Result<bool> {
    let mut list = read_custom_sources(data_paths);
    let before = list.len();
    list.retain(|c| c.kind != kind);
    if list.len() == before {
        return Ok(false);
    }
    write_custom_sources(data_paths, &list)?;
    let _ = store::remove_uninstalled_kind(data_paths, kind);
    checks::forget_source(data_paths, kind);
    Ok(true)
}

/// Checks a GitHub repository over the network, validating that it publishes
/// compatible `.tar.*` releases and discovering selectable variants. Blocking.
pub fn probe_github_source(data_paths: &DataPaths, remote: &dyn Remote, input: &str, now: u64) -> Result<ProbeResult> {
    let github = parse_github_repo(input)?;
    if let Some(existing) = existing_by_github(data_paths, &github) {
        return Err(anyhow!(tr!("This repository is already added as {name}.", name = existing)));
    }
    let temp_source = Source::Custom(CustomSource { kind: github.clone(), github: github.clone(), asset_filter: None });
    let json = match checks::fetch_releases_for_probe(data_paths, remote, &temp_source, PROBE_RELEASES, now) {
        Ok(body) => body,
        Err(ApiError::RateLimited { .. }) => {
            return Err(anyhow!(tr!("GitHub's rate limit was reached. Please try again later.")));
        }
        Err(ApiError::Other(e)) => return Err(anyhow!("{e}")),
    };
    let suggested_name = suggest_name(data_paths, &github);
    probe_from_releases(&github, &suggested_name, &json, cfg!(target_arch = "aarch64"))
}

/// Pure probe logic over a raw GitHub releases JSON payload.
pub fn probe_from_releases(github: &str, suggested_name: &str, json: &str, arm: bool) -> Result<ProbeResult> {
    let list: Vec<GhRelease> = serde_json::from_str(json).context("unexpected GitHub releases response")?;
    if list.is_empty() {
        return Err(anyhow!(tr!("This repository has no GitHub releases.")));
    }
    let downloadable_count = list.iter().filter(|r| pick_custom_archive(&r.assets, None, arm).is_some()).count();
    if downloadable_count == 0 {
        return Err(anyhow!(tr!("No compatible .tar.gz, .tar.xz, or .tar.zst builds found in this repository's releases.")));
    }
    // Prefer the newest stable release with builds; fall back to the newest pre-release.
    let sample = list
        .iter()
        .find(|r| !r.prerelease && pick_custom_archive(&r.assets, None, arm).is_some())
        .or_else(|| list.iter().find(|r| pick_custom_archive(&r.assets, None, arm).is_some()))
        .expect("downloadable_count > 0");

    let variants = discover_variants(&sample.assets, arm);
    Ok(ProbeResult {
        github: github.to_owned(),
        suggested_name: suggested_name.to_owned(),
        newest_tag: sample.tag_name.clone(),
        release_count: downloadable_count,
        variants,
        raw_releases: json.to_owned(),
    })
}

pub(crate) fn is_custom_archive(name: &str) -> bool {
    CUSTOM_ARCHIVES.iter().any(|ext| name.ends_with(ext))
}

pub(crate) fn strip_custom_archive_ext(name: &str) -> &str {
    CUSTOM_ARCHIVES.iter().find_map(|ext| name.strip_suffix(ext)).unwrap_or(name)
}

fn is_arm_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("arm64") || lower.contains("aarch64")
}

/// Penalty score for default selection on a custom repo (lower = more standard).
fn default_penalty(name: &str) -> (u32, usize) {
    let lower = name.to_ascii_lowercase();
    let mut p = 0u32;
    if ["debug", "symbols", "source", "-src.", "_src."].iter().any(|t| lower.contains(t)) {
        p += 100;
    }
    if lower.contains("wow64") {
        p += 20;
    }
    if ["_v3.", "-v3.", "_v3-", "-v3-", "_v4.", "-v4.", "_v4-", "-v4-", "x86_64_v3", "x86-64-v3", "x86_64_v4", "x86-64-v4"]
        .iter()
        .any(|t| lower.contains(t))
    {
        p += 15;
    }
    if lower.contains("-async") || lower.contains("_async") {
        p += 10;
    }
    if lower.contains("-native") || lower.contains("_native") {
        p += 5;
    }
    (p, name.len())
}

fn arch_archives(assets: &[GhAsset], arm: bool) -> Vec<&GhAsset> {
    assets
        .iter()
        .filter(|a| is_custom_archive(&a.name))
        .filter(|a| if arm { is_arm_name(&a.name) } else { !is_arm_name(&a.name) })
        .collect()
}

/// Picks the archive for a custom source on this machine.
pub(crate) fn pick_custom_archive<'a>(assets: &'a [GhAsset], filter: Option<&str>, arm: bool) -> Option<&'a GhAsset> {
    let candidates = arch_archives(assets, arm);
    if candidates.is_empty() {
        return None;
    }
    if let Some(token) = filter.filter(|s| !s.is_empty()) {
        return candidates.into_iter().find(|a| a.name.contains(token));
    }
    candidates.into_iter().min_by_key(|a| default_penalty(&a.name))
}

/// Discovers selectable archive variants for the probe preview. Index 0 is
/// always the automatic default; additional entries appear only when a release
/// ships multiple arch-compatible archives with distinguishable suffixes.
fn discover_variants(assets: &[GhAsset], arm: bool) -> Vec<ProbeVariant> {
    let candidates = arch_archives(assets, arm);
    let Some(default) = pick_custom_archive(assets, None, arm) else {
        return Vec::new();
    };
    let mut out = vec![ProbeVariant { label: tr!("Default").to_owned(), filter: None, example_file: default.name.clone() }];
    if candidates.len() <= 1 {
        return out;
    }
    let stems: Vec<&str> = candidates.iter().map(|a| strip_custom_archive_ext(&a.name)).collect();
    let prefix_len = common_prefix_len(&stems);
    let mut seen_tokens = HashSet::new();
    for (asset, stem) in candidates.iter().zip(&stems) {
        if asset.name == default.name {
            continue;
        }
        let Some((label, token)) = variant_token(stem, prefix_len) else { continue };
        // Keep only tokens that uniquely identify this archive among the release's candidates.
        let matches = candidates.iter().filter(|a| a.name.contains(&token)).count();
        if matches == 1 && seen_tokens.insert(token.clone()) {
            out.push(ProbeVariant { label, filter: Some(token), example_file: asset.name.clone() });
        }
    }
    out
}

fn common_prefix_len(items: &[&str]) -> usize {
    let Some((first, rest)) = items.split_first() else { return 0 };
    let mut len = first.len();
    for s in rest {
        len = first.bytes().zip(s.bytes()).take(len).take_while(|(a, b)| a == b).count();
    }
    // Keep UTF-8 boundary.
    while len > 0 && !first.is_char_boundary(len) {
        len -= 1;
    }
    len
}

/// Builds `(label, filter_token)` from the distinguishing tail of `stem`.
fn variant_token(stem: &str, prefix_len: usize) -> Option<(String, String)> {
    let tail = &stem[prefix_len..];
    if tail.is_empty() {
        return None;
    }
    // If the divergence is already on a separator (`_v3`, `-async`), keep it;
    // otherwise back up to the preceding `-` or `_` so tokens like `-v3` stay
    // whole when sibling archives share `-v`.
    let (sep, label) = if let Some(rest) = tail.strip_prefix('-') {
        (Some(b'-'), rest)
    } else if let Some(rest) = tail.strip_prefix('_') {
        (Some(b'_'), rest)
    } else {
        let before = &stem[..prefix_len];
        let start = before.rfind(['-', '_']).map(|i| i + 1).unwrap_or(prefix_len);
        (stem.as_bytes().get(start.saturating_sub(1)).copied(), &stem[start..])
    };
    let label = label.trim_matches(['-', '_']);
    if label.is_empty() {
        return None;
    }
    let token = match sep {
        Some(b'-') => format!("-{label}.tar"),
        Some(b'_') => format!("_{label}.tar"),
        _ => format!("{label}.tar"),
    };
    Some((label.to_owned(), token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::tests::Fake;

    const SALKIM: &str = r#"[
      {
        "tag_name": "wineland-11.0-20261124",
        "published_at": "2026-11-24T13:18:15Z",
        "html_url": "https://github.com/nanomatters/proton-salkim/releases/tag/wineland-11.0-20261124",
        "prerelease": false,
        "assets": [
          {"name": "proton-wineland-11.0-20261124-x86_64.sha512sum", "browser_download_url": "https://x/s.sha512sum", "size": 174},
          {"name": "proton-wineland-11.0-20261124-x86_64.tar.xz", "browser_download_url": "https://x/std.tar.xz", "size": 366165612},
          {"name": "proton-wineland-11.0-20261124-x86_64_v3.sha512sum", "browser_download_url": "https://x/v3.sha512sum", "size": 177},
          {"name": "proton-wineland-11.0-20261124-x86_64_v3.tar.xz", "browser_download_url": "https://x/v3.tar.xz", "size": 359914728},
          {"name": "proton-wineland-11.0-20261124-x86_64_wow64.sha512sum", "browser_download_url": "https://x/w.sha512sum", "size": 180},
          {"name": "proton-wineland-11.0-20261124-x86_64_wow64.tar.xz", "browser_download_url": "https://x/w.tar.xz", "size": 332734276}
        ]
      }
    ]"#;

    #[test]
    fn parses_github_urls_and_shorthand() {
        assert_eq!(parse_github_repo("nanomatters/proton-salkim").unwrap(), "nanomatters/proton-salkim");
        assert_eq!(parse_github_repo("https://github.com/nanomatters/proton-salkim").unwrap(), "nanomatters/proton-salkim");
        assert_eq!(parse_github_repo("https://github.com/nanomatters/proton-salkim/").unwrap(), "nanomatters/proton-salkim");
        assert_eq!(parse_github_repo("https://github.com/nanomatters/proton-salkim.git").unwrap(), "nanomatters/proton-salkim");
        assert_eq!(parse_github_repo("https://github.com/nanomatters/proton-salkim/releases/latest").unwrap(), "nanomatters/proton-salkim");
        assert_eq!(parse_github_repo("github.com/nanomatters/proton-salkim?tab=readme").unwrap(), "nanomatters/proton-salkim");
        assert!(parse_github_repo("").is_err());
        assert!(parse_github_repo("only-owner").is_err());
        assert!(parse_github_repo("https://gitlab.com/foo/bar").is_err());
        assert!(parse_github_repo("gitlab.com/foo/bar").is_err());
        assert!(parse_github_repo("../evil").is_err());
    }

    #[test]
    fn probes_salkim_variants_and_picks_standard_default() {
        let probe = probe_from_releases("nanomatters/proton-salkim", "proton-salkim", SALKIM, false).unwrap();
        assert_eq!(probe.newest_tag, "wineland-11.0-20261124");
        assert_eq!(probe.release_count, 1);
        assert_eq!(probe.variants.len(), 3);
        assert_eq!(probe.variants[0].filter, None);
        assert_eq!(probe.variants[0].example_file, "proton-wineland-11.0-20261124-x86_64.tar.xz");
        assert_eq!(probe.variants[1].filter.as_deref(), Some("_v3.tar"));
        assert_eq!(probe.variants[2].filter.as_deref(), Some("_wow64.tar"));

        // Selecting a variant token parses the matching archive and checksum.
        let src = Source::Custom(CustomSource {
            kind: "Proton-Wineland".into(),
            github: "nanomatters/proton-salkim".into(),
            asset_filter: probe.variants[1].filter.clone(),
        });
        let releases = repos::parse_releases_for_source(&src, SALKIM, false).unwrap();
        assert_eq!(releases[0].version, "Proton-Wineland-latest");
        assert_eq!(releases[1].version, "Proton-Wineland-wineland-11.0-20261124");
        assert_eq!(releases[1].download.as_deref(), Some("https://x/v3.tar.xz"));
        assert_eq!(releases[1].checksum.as_deref(), Some("https://x/v3.sha512sum"));
    }

    #[test]
    fn add_remove_and_duplicate_checks() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(tmp.path());

        let added = add_custom_source(
            &paths,
            CustomSource {
                kind: "Proton-Wineland".into(),
                github: "https://github.com/nanomatters/proton-salkim".into(),
                asset_filter: None,
            },
            Some(SALKIM),
        )
        .unwrap();
        assert_eq!(added.github, "nanomatters/proton-salkim");
        assert_eq!(read_custom_sources(&paths).len(), 1);
        assert!(store::read_releases(&paths).iter().any(|r| r.version == "Proton-Wineland-latest"));

        // Duplicate repo or name rejected.
        assert!(add_custom_source(
            &paths,
            CustomSource { kind: "Other".into(), github: "NanoMatters/proton-salkim".into(), asset_filter: None },
            None,
        )
        .is_err());
        assert!(add_custom_source(
            &paths,
            CustomSource { kind: "proton-wineland".into(), github: "someone/other".into(), asset_filter: None },
            None,
        )
        .is_err());
        // Built-in duplicate rejected.
        assert!(add_custom_source(
            &paths,
            CustomSource { kind: "My-GE".into(), github: "GloriousEggroll/proton-ge-custom".into(), asset_filter: None },
            None,
        )
        .is_err());
        // Name starting with Wine- rejected.
        assert!(add_custom_source(&paths, CustomSource { kind: "Wine-Foo".into(), github: "foo/bar".into(), asset_filter: None }, None,)
            .is_err());

        // Removing prunes uninstalled cached releases.
        assert!(remove_custom_source(&paths, "Proton-Wineland").unwrap());
        assert!(read_custom_sources(&paths).is_empty());
        assert!(store::read_releases(&paths).is_empty());
    }

    #[test]
    fn probe_through_remote_checks_duplicates_and_records_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DataPaths::at(tmp.path());
        let gh = Fake::new("<feed></feed>");
        gh.answer(Ok(SALKIM.into()), 50);

        // Built-in repo fails before touching the network.
        assert!(probe_github_source(&paths, &gh, "GloriousEggroll/proton-ge-custom", 1000).is_err());
        assert!(gh.asked.lock().unwrap().is_empty());

        let res = probe_github_source(&paths, &gh, "https://github.com/nanomatters/proton-salkim", 1000).unwrap();
        assert_eq!(res.suggested_name, "proton-salkim");
        assert_eq!(gh.asked.lock().unwrap().len(), 1);
    }
}

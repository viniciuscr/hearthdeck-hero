//! Where the fork touches upstream code: one table, and the tests that keep it
//! true.
//!
//! The Watch space (films and series) is a fork-owned subsystem, but it has to
//! plug into upstream where upstream dispatches. That "where" is this table:
//! the upstream files the page and the space's dispatch rely on, the fork's own
//! lines in them, and the `docs/FORK.md` row that justifies each. The tests fail
//! when the table, the tree and the register disagree, so a hook cannot be added
//! silently and a rename on either side cannot rot the register.
//!
//! The two rules the guards enforce:
//!
//! - **upstream's detail-page layouts never dispatch the fork's page.** They may
//!   word things per space (`docs/FORK.md` F-016, F-021); they may not call the
//!   page. The page is answered from `console/game/mod.rs`, its one dispatch
//!   seam.
//! - **every file the tree marks with a `Fork-Seam` trailer is in the table**,
//!   so the table cannot shrink behind the markers.
//!
//! Test-only: this is documentation the build checks, not something the running
//! app reads (`mod seams` is `#[cfg(test)]`). `docs/WATCH-PAGE.md` is the
//! decision it serves.

/// One upstream file the fork edits, why, and where the register records it.
#[derive(Debug)]
struct Seam {
    /// Workspace-relative path of the upstream-owned file.
    pub file: &'static str,
    /// The `docs/FORK.md` rows that justify the edit.
    pub register: &'static [&'static str],
    /// The fork's own lines: text that only the fork wrote. A rename on either
    /// side breaks the test, which is the point — the table is the source of
    /// truth for the deviation surface.
    pub sites: &'static [&'static str],
    /// What the file carries, in one line.
    pub why: &'static str,
}

/// Every seam of the Watch space, page first.
const SEAMS: &[Seam] = &[
    Seam {
        file: "app/src/collection/console/game/mod.rs",
        register: &["F-012", "F-010"],
        sites: &[
            "Fork-Seam: F-012",
            "watch_console_rows",
            "self.space == super::super::Space::Watch",
            "pub(super) use hero::facts_table",
            "fn classic_facts",
        ],
        why: "the detail page's dispatch: the seven questions the shell asks (the space branch answers them), and the facts table the page reuses",
    },
    Seam {
        file: "app/src/collection/console/mod.rs",
        register: &["F-012", "F-010", "F-016"],
        sites: &[
            "mod watch_page;",
            "mod watch_panel;",
            "console_trailer_hint",
            "watch_open_title",
            "watch_feed_start",
            "space == Space::Watch",
        ],
        why:
            "the shell: the page's modules, the L3 trailer press, opening a title from a cover, the details debounce and the space's reset",
    },
    Seam {
        file: "app/src/collection/mod.rs",
        register: &["F-006", "F-008"],
        sites: &["mod watch_catalog;", "mod watch_facts;", "mod watch_personal;", "mod watch_playback;", "mod watch_service;"],
        why: "the space itself: its modules and the view state the Collection keeps for it",
    },
    Seam {
        file: "app/src/collection/console/game/tabs.rs",
        register: &["F-016"],
        sites: &["if self.space.capabilities().manage {"],
        why: "the Manage tab, which a space without installs or settings does not show",
    },
    Seam {
        file: "app/src/collection/data.rs",
        register: &["F-021"],
        sites: &["Fork-Seam: F-021", "watch_facts::relabel"],
        why: "the focus view model's rows are worded by the space (Last watched, Watched)",
    },
    Seam {
        file: "app/src/collection/actions.rs",
        register: &["F-006"],
        sites: &["Fork-Seam: F-006"],
        why: "the focus is built for the space on screen, so a film's rows are its own",
    },
    Seam {
        file: "app/src/collection/console/game/hero.rs",
        register: &["F-012"],
        sites: &["pub(crate) fn facts_table"],
        why: "the facts table, whose visibility the film page needs (upstream keeps it to the console module)",
    },
    Seam {
        file: "app/src/collection/console/hero.rs",
        register: &["F-016", "F-021"],
        sites: &["pub(crate) fn watched_or_played"],
        why: "the hero's lines say watched or played, and name the time the space keeps",
    },
    Seam {
        file: "app/src/collection/console/tile.rs",
        register: &["F-019", "F-021"],
        sites: &["crate::fork::watch::sources::dimmed", "crate::collection::Space::Watch"],
        why: "a cover's grey state and its caption follow the space",
    },
    Seam {
        file: "app/src/collection/console/layouts/carousel.rs",
        register: &["F-008", "F-019"],
        sites: &["Fork-Seam: F-008", "crate::fork::watch::sources::dimmed"],
        why: "the focused cover's caption says watched, and a title nothing can play stays grey",
    },
    Seam {
        file: "app/src/collection/console/layouts/cinema.rs",
        register: &["F-008", "F-016"],
        sites: &["Fork-Seam: F-008", "watched_or_played(self.space, &time)"],
        why: "the page's facts say watched, not played",
    },
    Seam {
        file: "app/src/collection/console/layouts/grid.rs",
        register: &["F-008", "F-021"],
        sites: &["Fork-Seam: F-008", "fn sort_labels"],
        why: "the grid's three Watch filters and its sort chip labels",
    },
    Seam {
        file: "app/src/collection/console/layouts/spotlight/mod.rs",
        register: &["F-012", "F-010"],
        sites: &["Fork-Seam: F-012", "played_line(self.space, g, true)", "watch_facts::library_line"],
        why: "the film page's own line and its trailer hint",
    },
    Seam {
        file: "app/src/collection/console/layouts/spotlight/parts.rs",
        register: &["F-008", "F-021"],
        sites: &["Fork-Seam: F-008", "self.space == crate::collection::Space::Watch"],
        why: "the facts panel says watched, and names the length as a film's or a series'",
    },
    Seam {
        file: "app/src/collection/hltb_ui.rs",
        register: &["F-012"],
        sites: &["Fork-Seam: F-012", "self.space == super::Space::Watch"],
        why: "HowLongToBeat knows games, so it is never asked about a film",
    },
    Seam {
        file: "app/src/console/theme_rows.rs",
        register: &["F-008", "F-016"],
        sites: &["Fork-Seam: F-008", "crate::fork::watch::prefs::preset(&prefs.based_on)"],
        why: "Watch starts from Classic, which upstream no longer offers as a theme",
    },
    Seam {
        file: "app/src/pages/settings/console/mod.rs",
        register: &["F-006", "F-008"],
        sites: &["Fork-Seam: F-008", "CSec::Rows(Section::Watch)"],
        why: "Settings → Watch: the space's own sections",
    },
    Seam {
        file: "app/src/collection/focus_left.rs",
        register: &["F-022"],
        sites: &["self.space.capabilities()"],
        why: "the desktop focus panel drops the game-only rows (install, settings, playtime)",
    },
    Seam {
        file: "app/src/collection/context_menu.rs",
        register: &["F-022"],
        sites: &["capabilities().install"],
        why: "the desktop context menu offers no install or manage verbs for a film",
    },
    Seam {
        file: "app/src/main.rs",
        register: &["F-022"],
        sites: &["mod fork;"],
        why: "the fork module root: every fork-owned module hangs from it",
    },
    Seam {
        file: "app/src/console/mod.rs",
        register: &["F-012"],
        sites: &["/// The click of the left stick (L3).", "/// The click of the right stick (R3)."],
        why: "the Pad and Btn variants for the two stick clicks the page's trailer and watch-later presses use",
    },
    Seam {
        file: "app/src/console/input.rs",
        register: &["F-012"],
        sites: &["B::LeftStick => Pad::L3,", "B::RightStick => Pad::R3,"],
        why: "the input stack reports both stick clicks",
    },
    Seam {
        file: "app/src/console/input/gilrs_pads.rs",
        register: &["F-012"],
        sites: &["B::LeftThumb => PadButton::LeftStick,"],
        why: "gilrs reports the left stick's click",
    },
    Seam {
        file: "app/src/console/input/sdl_pads.rs",
        register: &["F-012"],
        sites: &["Button::LeftStick => PadButton::LeftStick,"],
        why: "SDL reports the left stick's click",
    },
    Seam {
        file: "app/src/console/kit.rs",
        register: &["F-012"],
        sites: &["Btn::L3 => Mark::Cap", "Btn::R3 => Mark::Cap"],
        why: "the glyphs and the key cap for both stick clicks",
    },
    Seam {
        file: "app/src/console/sound.rs",
        register: &["F-012"],
        sites: &["Pad::L3 | Pad::R3 => Some(Sound::Confirm)"],
        why: "a stick click confirms like every other press",
    },
    Seam {
        file: "crates/platform/src/controllers.rs",
        register: &["F-012"],
        sites: &["    LeftStick,", "    RightStick,"],
        why: "every controller backend knows the two stick buttons",
    },
];

/// The fork's own page: the one place allowed to name the page's entry points.
const PAGE: &str = "app/src/collection/console/watch_page";

/// The page's entry points. Only the page itself and its dispatch seam call
/// them; a layout body that does is the drift this module exists to stop.
const PAGE_CALLS: &[&str] = &["render_watch_game_page", "watch_page_pad", "watch_page_hints"];

/// The directory of upstream's own detail page: its dispatch file answers the
/// shell, every other file in it is a layout body and stays upstream's.
const GAME_PAGE: &str = "app/src/collection/console/game";

/// The layouts a tab draws from: per-space wording allowed, page dispatch not.
const LAYOUTS: &str = "app/src/collection/console/layouts";

/// The first line of the paragraph in `docs/FORK.md` that enumerates the seams.
const REGISTER_INTRO: &str = "The machine-checked list of those seams is";

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// This table's own path: skipped by the scans that read the tree.
    const TABLE: &str = "app/src/fork/seams.rs";

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("the app crate lives in the workspace").to_path_buf()
    }

    fn read(relative: &str) -> String {
        let path = repo_root().join(relative);
        std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()))
    }

    /// Every `.rs` file under `dir`, with its workspace-relative path. Symlinks
    /// are neither followed nor descended into.
    fn rust_files(relative: &str, out: &mut Vec<(String, String)>) {
        let dir = repo_root().join(relative);
        let entries = std::fs::read_dir(&dir).unwrap_or_else(|error| panic!("listing {}: {error}", dir.display()));
        for entry in entries {
            let entry = entry.expect("a readable directory entry");
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let child = format!("{relative}/{name}");
            match entry.file_type().expect("a readable file type") {
                t if t.is_dir() => rust_files(&child, out),
                t if t.is_file() && name.ends_with(".rs") => {
                    let body = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
                    out.push((child, body));
                }
                _ => {}
            }
        }
    }

    /// The seam files `docs/FORK.md` itself enumerates, as backticked `.rs`
    /// paths in the paragraph `REGISTER_INTRO` opens. That sentence also names
    /// this table, so its own path is skipped.
    fn register_files() -> Vec<String> {
        let fork = read("docs/FORK.md");
        let paragraph = fork
            .lines()
            .skip_while(|line| !line.starts_with(REGISTER_INTRO))
            .take_while(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        paragraph.split('`').filter(|token| token.ends_with(".rs") && *token != TABLE).map(str::to_owned).collect()
    }

    /// Every seam is in the register's own list, and every row it cites exists.
    #[test]
    fn every_seam_is_in_the_register() {
        let listed = register_files();
        let fork = read("docs/FORK.md");
        for seam in SEAMS {
            assert!(listed.iter().any(|path| path == seam.file), "{} is a seam but the register's list omits it", seam.file);
            for row in seam.register {
                assert!(fork.contains(&format!("| {row} |")), "{} cites {row}, which docs/FORK.md does not have: {}", seam.file, seam.why);
            }
        }
    }

    /// And the register lists nothing the table does not carry, so the two
    /// cannot drift apart in either direction.
    #[test]
    fn the_register_lists_no_seam_the_table_lacks() {
        for path in register_files() {
            assert!(SEAMS.iter().any(|seam| seam.file == path), "docs/FORK.md lists {path} as a seam, but the table does not carry it");
        }
    }

    /// Every file the tree marks with a `Fork-Seam` trailer is in the table: a
    /// new hook cannot appear without its row. The table is skipped — it names
    /// the markers as its tokens, it does not carry one.
    #[test]
    fn every_trailer_is_in_the_table() {
        let mut files = Vec::new();
        rust_files("app/src", &mut files);
        rust_files("crates", &mut files);
        for (file, body) in files {
            if file == TABLE {
                continue;
            }
            let marks = body.lines().any(|line| line.trim_start().trim_start_matches('/').trim_start().starts_with("Fork-Seam:"));
            if marks {
                assert!(SEAMS.iter().any(|seam| seam.file == file), "{file} carries a Fork-Seam trailer but is not in the seam table");
            }
        }
    }

    /// The lines the table points at still exist, so a rename on either side of
    /// the boundary fails here instead of at the next reconciliation.
    #[test]
    fn every_seam_site_still_exists() {
        for seam in SEAMS {
            let body = read(seam.file);
            for site in seam.sites {
                assert!(
                    body.contains(site),
                    "{} no longer contains {site:?} — update the seam table and docs/FORK.md ({})",
                    seam.file,
                    seam.why
                );
            }
        }
    }

    /// Upstream's detail-page layouts never dispatch the fork's page: the page
    /// is answered from its own seam (`game/mod.rs`, the one file allowed to).
    #[test]
    fn upstream_layouts_never_dispatch_the_fork_page() {
        let mut files = Vec::new();
        rust_files(GAME_PAGE, &mut files);
        for (file, body) in files {
            if file == format!("{GAME_PAGE}/mod.rs") {
                continue;
            }
            assert!(!body.contains("Space::Watch"), "{file} branches on the space; answer the shell from the page's dispatch seam instead");
            for call in PAGE_CALLS {
                assert!(!body.contains(call), "{file} calls the fork's page ({call}); that belongs in {GAME_PAGE}/mod.rs");
            }
        }

        // The layout bodies keep their registered per-space wording (F-016,
        // F-021), but must not dispatch a page either.
        let mut files = Vec::new();
        rust_files(LAYOUTS, &mut files);
        for (file, body) in files {
            for call in PAGE_CALLS {
                assert!(!body.contains(call), "{file} calls the fork's page ({call}); the page is a screen of its own");
            }
        }
    }

    /// The page is named only by itself and by its dispatch seam.
    #[test]
    fn the_film_page_is_referenced_only_at_its_seam() {
        let dispatch = format!("{GAME_PAGE}/mod.rs");
        let mut files = Vec::new();
        rust_files("app/src/collection/console", &mut files);
        for (file, body) in files {
            if file == dispatch || file.starts_with(PAGE) {
                continue;
            }
            for call in PAGE_CALLS {
                assert!(
                    !body.contains(call),
                    "{file} reaches the film page ({call}); keep the page behind its seam so upstream's layouts stay mergeable"
                );
            }
        }
    }

    /// The page's entry points are named where the guards look for them: the
    /// dispatch seam. A rename there fails here instead of leaving the guards
    /// above matching nothing.
    #[test]
    fn the_dispatch_seam_names_every_page_entry_point() {
        let dispatch = read(&format!("{GAME_PAGE}/mod.rs"));
        for call in PAGE_CALLS {
            assert!(
                dispatch.contains(call),
                "{GAME_PAGE}/mod.rs no longer names {call}: update PAGE_CALLS, the seam table and every caller"
            );
        }
    }

    /// The dispatch answers the shell from seven places (six `==` branches and
    /// the `!=` one that keeps the tab bar). One token covers them all, so its
    /// count is what proves none was quietly dropped; `docs/WATCH-PAGE.md` says
    /// three of them fold into a page value in Phase 2, which is the moment to
    /// change this number.
    #[test]
    fn the_dispatch_seam_answers_seven_shell_questions() {
        let dispatch = read(&format!("{GAME_PAGE}/mod.rs"));
        let branches = dispatch.matches("super::super::Space::Watch").count();
        assert_eq!(branches, 7, "{GAME_PAGE}/mod.rs answers the shell from {branches} places, not 7 — update the count with the design");
    }
}

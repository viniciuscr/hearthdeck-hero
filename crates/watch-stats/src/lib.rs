//! What was watched, when: the numbers a recap is made of.
//!
//! The games' Statistics page is built from `local_library/sessions.json`
//! (`crates/data/src/stats`); this is Watch's counterpart, over the sessions
//! `heronite-watch-state` records at the end of each playback. Deliberately not
//! games-shaped: no statuses, no backlog goals, no journal — a film is finished
//! or it is not, and a series is episodes.
//!
//! Pure and blocking-free: the app hands it the sessions and a date, and gets
//! buckets and totals back to draw. Reuse over invention: a *span* is a period
//! ending today, a *bucket* is a day, and a *title row* is one title's share.

use chrono::{Datelike, Duration, NaiveDate, Timelike, Utc};
use heronite_watch_state::sessions::WatchSession;

pub use heronite_watch_state::sessions::WatchSession as Session;

/// The window the person is looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Week,
    Month,
    Year,
    All,
}

impl Span {
    pub const ALL: [Span; 4] = [Span::Week, Span::Month, Span::Year, Span::All];
}

/// A window, `[start, end)`, days in the viewer's own timezone's terms (the
/// sessions are UTC; a recap that splits days at UTC midnight would move a
/// Saturday night film to Sunday for half the world, so the caller passes the
/// local date and the comparison is done on UTC dates as stored — one day of
/// slack at the edges is not worth a timezone database here).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Period {
    pub span: Span,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

/// The window `span` covers, ending tomorrow (so today is inside it).
pub fn period(span: Span, today: NaiveDate) -> Period {
    let end = today + Duration::days(1);
    let start = match span {
        Span::Week => today - Duration::days(i64::from(today.weekday().num_days_from_monday())),
        Span::Month => NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today),
        Span::Year => NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today),
        Span::All => NaiveDate::MIN,
    };
    Period { span, start, end }
}

/// How much of a window was watched, and how it splits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals {
    pub sessions: usize,
    /// Distinct titles (a series counts once, whatever its episodes).
    pub titles: usize,
    pub films: usize,
    pub series: usize,
    pub seconds: f64,
    /// Sessions that reached the finished threshold.
    pub finished: usize,
    /// Sessions that stopped short of it.
    pub unfinished: usize,
}

impl Totals {
    pub fn hours(&self) -> f64 {
        self.seconds / 3600.
    }
}

/// One day of the window, for the bars.
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub seconds: f64,
    pub sessions: usize,
}

/// One title's share of the window.
#[derive(Clone, Debug, PartialEq)]
pub struct TitleRow {
    /// Provider and id: what tells two remakes apart.
    pub provider: String,
    pub id: String,
    pub kind: String,
    pub title: String,
    /// The newest episode watched, when it is a series.
    pub video_id: Option<String>,
    pub seconds: f64,
    pub sessions: usize,
    /// Some session of this title reached the end.
    pub finished: bool,
}

impl TitleRow {
    pub fn hours(&self) -> f64 {
        self.seconds / 3600.
    }
}

/// The whole picture for one window.
#[derive(Clone, Debug, PartialEq)]
pub struct Recap {
    pub period: Period,
    pub totals: Totals,
    /// Every day of the window, in order, empty days included: a bar chart with
    /// holes in it is a lie.
    pub days: Vec<Day>,
    /// The window's titles, most watched first.
    pub titles: Vec<TitleRow>,
    /// The longest run of consecutive days with something watched, inside the
    /// window, and the day it ended on.
    pub longest_streak: (usize, Option<NaiveDate>),
    /// The current run, ending today or yesterday (a day not over yet is not a
    /// broken streak).
    pub current_streak: usize,
    /// Days with three or more sessions.
    pub binge_days: usize,
    /// Sessions that started before 06:00, and after 18:00, as the person's
    /// clock reads them.
    pub early: usize,
    pub evening: usize,
}

impl Recap {
    pub fn busiest_day(&self) -> Option<&Day> {
        self.days.iter().filter(|day| day.sessions > 0).max_by(|a, b| a.seconds.total_cmp(&b.seconds))
    }
}

/// The recap of `sessions` in `period`. Sessions outside it are ignored, and an
/// empty window is a valid, empty recap.
pub fn compute(sessions: &[WatchSession], period: Period) -> Recap {
    let mut days: Vec<Day> = Vec::new();
    if period.span != Span::All {
        let mut date = period.start;
        while date < period.end {
            days.push(Day { date, seconds: 0., sessions: 0 });
            date += Duration::days(1);
        }
    }
    let mut totals = Totals::default();
    let mut by_title: Vec<TitleRow> = Vec::new();
    let mut early = 0;
    let mut evening = 0;

    for session in sessions.iter().filter(|session| inside(session, period)) {
        let date = session.started_at.date_naive();
        match days.iter_mut().find(|day| day.date == date) {
            Some(day) => {
                day.seconds += session.elapsed_seconds;
                day.sessions += 1;
            }
            // `All`: the days are only the ones something happened on.
            None => days.push(Day { date, seconds: session.elapsed_seconds, sessions: 1 }),
        }
        totals.sessions += 1;
        totals.seconds += session.elapsed_seconds;
        if session.finished {
            totals.finished += 1;
        } else {
            totals.unfinished += 1;
        }
        let hour = session.started_at.with_timezone(&chrono::Local).hour();
        if hour < 6 {
            early += 1;
        } else if hour >= 18 {
            evening += 1;
        }
        match by_title.iter_mut().find(|row| row.provider == session.provider && row.id == session.id) {
            Some(row) => {
                row.seconds += session.elapsed_seconds;
                row.sessions += 1;
                row.finished |= session.finished;
                // The episode of the latest session is the one the row shows.
                if row.video_id.is_none() {
                    row.video_id = session.video_id.clone();
                }
            }
            None => by_title.push(TitleRow {
                provider: session.provider.clone(),
                id: session.id.clone(),
                kind: session.kind.clone(),
                title: session.title.clone(),
                video_id: session.video_id.clone(),
                seconds: session.elapsed_seconds,
                sessions: 1,
                finished: session.finished,
            }),
        }
    }

    days.sort_by_key(|day| day.date);
    by_title.sort_by(|a, b| b.seconds.total_cmp(&a.seconds).then_with(|| a.title.cmp(&b.title)));
    totals.titles = by_title.len();
    totals.films = by_title.iter().filter(|row| row.kind != "series").count();
    totals.series = by_title.iter().filter(|row| row.kind == "series").count();
    let (longest_streak, current_streak) = streaks(&days);
    let binge_days = days.iter().filter(|day| day.sessions >= 3).count();
    Recap { period, totals, days, titles: by_title, longest_streak, current_streak, binge_days, early, evening }
}

fn inside(session: &WatchSession, period: Period) -> bool {
    let date = session.started_at.date_naive();
    date >= period.start && date < period.end
}

/// The longest run of consecutive watched days, and the run still alive at the
/// window's end (a day that has not happened yet does not break it).
fn streaks(days: &[Day]) -> ((usize, Option<NaiveDate>), usize) {
    let (mut longest, mut longest_end) = (0, None);
    let mut run = 0;
    for day in days {
        if day.sessions > 0 {
            run += 1;
            if run > longest {
                longest = run;
                longest_end = Some(day.date);
            }
        } else {
            run = 0;
        }
    }
    // The current streak walks back from the last day with something watched.
    let mut current = 0;
    for day in days.iter().rev() {
        if day.sessions > 0 {
            current += 1;
        } else {
            break;
        }
    }
    ((longest, longest_end), current)
}

/// The days folded into months, for a year's bars: `(1..=12, seconds)`, in order,
/// including the months that have nothing yet.
pub fn months(days: &[Day]) -> Vec<(u32, f64)> {
    let mut out: Vec<(u32, f64)> = (1..=12).map(|month| (month, 0.)).collect();
    for day in days {
        if let Some(entry) = out.iter_mut().find(|(month, _)| *month == day.date.month()) {
            entry.1 += day.seconds;
        }
    }
    out
}

/// Today, as the app's clock reads it.
pub fn today() -> NaiveDate {
    Utc::now().with_timezone(&chrono::Local).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone};

    fn at(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn session(id: &str, kind: &str, started: DateTime<Utc>, elapsed: f64, finished: bool) -> WatchSession {
        WatchSession {
            provider: "stremio".into(),
            kind: kind.into(),
            id: id.into(),
            video_id: (kind == "series").then(|| format!("{id}:1:1")),
            season: (kind == "series").then_some(1),
            number: (kind == "series").then_some(1),
            name: format!("Title {id}"),
            year: None,
            title: format!("Title {id}"),
            started_at: started,
            ended_at: started,
            elapsed_seconds: elapsed,
            position_from_ms: 0,
            position_to_ms: 0,
            duration_ms: 0,
            finished,
        }
    }

    #[test]
    fn a_month_has_a_bucket_for_every_day_something_or_nothing() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let period = period(Span::Month, today);
        assert_eq!(period.start, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());
        let sessions = [session("tt1", "movie", at(2026, 11, 3, 20), 7_200., true)];
        let recap = compute(&sessions, period);
        assert_eq!(recap.days.len(), 16, "the first of the month through today");
        assert_eq!(recap.days[2].date, NaiveDate::from_ymd_opt(2026, 11, 3).unwrap());
        assert!((recap.days[2].seconds - 7_200.).abs() < 1e-9);
        assert_eq!(recap.days[2].sessions, 1);
        assert_eq!(recap.totals.sessions, 1);
        assert!((recap.totals.hours() - 2.).abs() < 1e-9);
        assert_eq!((recap.totals.films, recap.totals.series), (1, 0));
        assert_eq!(recap.totals.finished, 1);
    }

    #[test]
    fn a_week_starts_on_monday_and_a_year_on_january_first() {
        let monday = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let week = period(Span::Week, monday);
        assert_eq!(week.start, monday);
        let sunday = NaiveDate::from_ymd_opt(2026, 11, 22).unwrap();
        assert_eq!(period(Span::Week, sunday).start, monday, "the same week");
        let year = period(Span::Year, monday);
        assert_eq!(year.start, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
        assert_eq!(year.end, NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(), "through today, tomorrow as the end");
    }

    #[test]
    fn sessions_outside_the_window_are_ignored() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let sessions =
            [session("tt1", "movie", at(2026, 10, 31, 22), 3_600., true), session("tt2", "movie", at(2026, 11, 2, 22), 1_800., false)];
        let recap = compute(&sessions, period(Span::Month, today));
        assert_eq!(recap.totals.sessions, 1, "October is not this month");
        assert_eq!(recap.titles.len(), 1);
        assert_eq!(recap.titles[0].id, "tt2");
    }

    #[test]
    fn all_time_counts_what_it_has_and_keeps_only_days_that_happened() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let sessions = [
            session("tt1", "movie", at(2024, 1, 5, 21), 3_600., true),
            session("tt1", "movie", at(2026, 11, 15, 21), 3_600., true),
            session("tt2", "series", at(2026, 11, 15, 22), 2_400., false),
        ];
        let recap = compute(&sessions, period(Span::All, today));
        assert_eq!(recap.totals.sessions, 3);
        assert_eq!(recap.totals.titles, 2, "a rewatch is not a second title");
        assert_eq!(recap.days.len(), 2, "only days with something");
        assert_eq!(recap.titles[0].id, "tt1", "most watched first");
        assert!((recap.titles[0].hours() - 2.).abs() < 1e-9);
        assert_eq!(recap.titles[0].sessions, 2);
        assert_eq!(recap.titles[1].kind, "series");
    }

    #[test]
    fn streaks_count_consecutive_days_and_a_binge_is_three_in_one() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let sessions = [
            session("a", "movie", at(2026, 11, 12, 20), 600., true),
            session("b", "movie", at(2026, 11, 13, 20), 600., true),
            session("c", "movie", at(2026, 11, 14, 20), 600., true),
            session("d", "movie", at(2026, 11, 14, 21), 600., true),
            session("e", "movie", at(2026, 11, 14, 22), 600., true),
            session("f", "movie", at(2026, 11, 16, 20), 600., true),
        ];
        let recap = compute(&sessions, period(Span::Month, today));
        assert_eq!(recap.longest_streak.0, 3, "the 12th, 13th and 14th");
        assert_eq!(recap.longest_streak.1, NaiveDate::from_ymd_opt(2026, 11, 14), "ending on the last of the run");
        assert_eq!(recap.current_streak, 1, "the 16th alone: the 15th had nothing");
        assert_eq!(recap.binge_days, 1, "three sessions on the 14th");
    }

    #[test]
    fn the_hour_of_the_day_is_read_on_the_local_clock() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let sessions = [session("a", "movie", at(2026, 11, 15, 3), 600., true), session("b", "movie", at(2026, 11, 15, 21), 600., true)];
        let recap = compute(&sessions, period(Span::Month, today));
        assert_eq!((recap.early, recap.evening), (1, 1), "03:00 and 21:00 as the viewer's clock reads them");
    }

    #[test]
    fn a_years_days_fold_into_twelve_months() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let sessions = [session("a", "movie", at(2026, 2, 3, 20), 3_600., true), session("b", "movie", at(2026, 2, 4, 20), 1_800., true)];
        let recap = compute(&sessions, period(Span::Year, today));
        let months = months(&recap.days);
        assert_eq!(months.len(), 12);
        assert!((months[1].1 - 5_400.).abs() < 1e-9, "February holds both");
        assert!((months[0].1).abs() < 1e-9, "January is empty, not missing");
    }

    #[test]
    fn an_empty_window_is_an_empty_recap() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let recap = compute(&[], period(Span::Week, today));
        assert_eq!(recap.totals.sessions, 0);
        // 2026-11-16 is a Monday: the week has only started, so it holds today.
        assert_eq!(recap.days.len(), 1);
        assert!(recap.titles.is_empty());
        assert_eq!(recap.longest_streak.0, 0);
        assert!(recap.busiest_day().is_none());
    }
}

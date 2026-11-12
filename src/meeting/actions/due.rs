//! A deadline as someone said it ("by Friday", "end of the month",
//! "November 20 at 3pm"), resolved against the day of the meeting.
//!
//! Only what names a day resolves. "Next week" or "soon" names none, so it
//! stays as said and nothing is guessed.

use chrono::{Datelike, Duration, NaiveDate, NaiveTime, Weekday};
use regex::Regex;
use std::sync::OnceLock;

/// A said deadline as a day, and the time of day when one was said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Due {
    pub on: NaiveDate,
    pub at: Option<NaiveTime>,
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

fn month(word: &str) -> Option<u32> {
    let w = word.trim_end_matches('.');
    if w.len() < 3 {
        return None;
    }
    MONTHS
        .iter()
        .position(|m| m.starts_with(w) && (w.len() >= 3))
        .filter(|&i| w == MONTHS[i] || w.len() == 3 || (i == 8 && w == "sept"))
        .map(|i| i as u32 + 1)
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// The first `day` after `from` (never `from` itself).
fn next_weekday(from: NaiveDate, day: Weekday) -> NaiveDate {
    let ahead = (day.num_days_from_monday() as i64 - from.weekday().num_days_from_monday() as i64)
        .rem_euclid(7);
    from + Duration::days(if ahead == 0 { 7 } else { ahead })
}

fn last_of_month(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1).unwrap() - Duration::days(1)
}

fn time_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(\d{1,2})(?::(\d{2}))?\s*(am|pm|a\.m\.|p\.m\.)|\b(\d{1,2}):(\d{2})\b|\b(noon|midday)\b")
            .unwrap()
    })
}

/// The time of day in `text`: "3pm", "3:30 pm", "15:00", "noon".
fn time_of_day(text: &str) -> Option<NaiveTime> {
    let c = time_re().captures(text)?;
    if c.get(6).is_some() {
        return NaiveTime::from_hms_opt(12, 0, 0);
    }
    if let Some(h) = c.get(1) {
        let mut h: u32 = h.as_str().parse().ok()?;
        let m: u32 = c.get(2).map_or(Some(0), |m| m.as_str().parse().ok())?;
        let pm = c.get(3)?.as_str().starts_with('p');
        if !(1..=12).contains(&h) {
            return None;
        }
        if pm && h != 12 {
            h += 12;
        } else if !pm && h == 12 {
            h = 0;
        }
        return NaiveTime::from_hms_opt(h, m, 0);
    }
    let h: u32 = c.get(4)?.as_str().parse().ok()?;
    let m: u32 = c.get(5)?.as_str().parse().ok()?;
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Resolve `said` against `meeting_day`, or `None` when it names no day.
pub fn resolve(said: &str, meeting_day: NaiveDate) -> Option<Due> {
    let text = said.trim().to_lowercase();
    let at = time_of_day(&text);
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',')
        .map(|w| w.trim_matches(|c: char| matches!(c, '.' | '!' | '?' | ';' | ':')))
        .filter(|w| !w.is_empty())
        .collect();
    let has = |w: &str| words.contains(&w);
    let phrase = |p: &str| text.contains(p);

    if let Ok(d) = NaiveDate::parse_from_str(text.split_whitespace().next()?, "%Y-%m-%d") {
        return Some(Due { on: d, at });
    }
    let on = |d: NaiveDate| Some(Due { on: d, at });

    if has("today")
        || has("tonight")
        || has("eod")
        || phrase("end of day")
        || phrase("end of the day")
        || phrase("this afternoon")
        || phrase("this morning")
        || phrase("this evening")
    {
        return on(meeting_day);
    }
    if has("tomorrow") {
        return on(meeting_day + Duration::days(1));
    }
    if phrase("end of the month") || phrase("end of month") || has("eom") {
        return on(last_of_month(meeting_day));
    }
    if phrase("end of the week") || phrase("end of week") || has("eow") {
        let friday = if meeting_day.weekday() == Weekday::Fri {
            meeting_day
        } else {
            next_weekday(meeting_day, Weekday::Fri)
        };
        return on(friday);
    }

    // "November 20", "Nov 20th", "20 November", "the 20th of November".
    for (i, w) in words.iter().enumerate() {
        let Some(m) = month(w) else { continue };
        let num = |s: &str| -> Option<u32> {
            let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            let rest = &s[d.len()..];
            (matches!(rest, "" | "st" | "nd" | "rd" | "th")).then_some(())?;
            d.parse().ok()
        };
        // "May" with no day next to it is as likely the verb.
        let Some(day) = words
            .get(i + 1)
            .and_then(|n| num(n))
            .or_else(|| i.checked_sub(1).and_then(|j| num(words[j])))
            .or_else(|| {
                (i >= 2 && words[i - 1] == "of")
                    .then(|| num(words[i - 2]))
                    .flatten()
            })
        else {
            continue;
        };
        let Some(mut date) = NaiveDate::from_ymd_opt(meeting_day.year(), m, day) else {
            continue;
        };
        if date < meeting_day {
            date = NaiveDate::from_ymd_opt(meeting_day.year() + 1, m, day).unwrap_or(date);
        }
        return on(date);
    }

    // "Friday", "by Friday", "next Friday". A bare weekday is the next one
    // after the meeting; "next" skips to the week after when that one falls
    // in the meeting's own week.
    for (i, w) in words.iter().enumerate() {
        let Some(day) = weekday(w) else { continue };
        let mut date = next_weekday(meeting_day, day);
        if i > 0 && words[i - 1] == "next" && date.iso_week() == meeting_day.iso_week() {
            date += Duration::days(7);
        }
        return on(date);
    }

    // A time alone ("by 3pm") is today's.
    at.map(|at| Due {
        on: meeting_day,
        at: Some(at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thursday, 12 November 2026.
    fn thu() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()
    }

    fn day(said: &str) -> Option<String> {
        resolve(said, thu()).map(|d| d.on.format("%Y-%m-%d").to_string())
    }

    fn time(said: &str) -> Option<String> {
        resolve(said, thu())
            .and_then(|d| d.at)
            .map(|t| t.format("%H:%M").to_string())
    }

    #[test]
    fn days_resolve_against_the_meeting() {
        assert_eq!(thu().weekday(), Weekday::Thu, "the fixture is a Thursday");
        assert_eq!(day("today").as_deref(), Some("2026-11-12"));
        assert_eq!(day("by end of day").as_deref(), Some("2026-11-12"));
        assert_eq!(day("EOD").as_deref(), Some("2026-11-12"));
        assert_eq!(day("Tomorrow").as_deref(), Some("2026-11-13"));
        assert_eq!(day("Friday").as_deref(), Some("2026-11-13"));
        assert_eq!(day("by Friday").as_deref(), Some("2026-11-13"));
        assert_eq!(day("next Friday").as_deref(), Some("2026-11-20"));
        assert_eq!(day("Monday").as_deref(), Some("2026-11-16"));
        assert_eq!(day("next Monday").as_deref(), Some("2026-11-16"));
        assert_eq!(day("Thursday").as_deref(), Some("2026-11-19"), "a week on");
        assert_eq!(day("end of the week").as_deref(), Some("2026-11-13"));
        assert_eq!(day("end of the month").as_deref(), Some("2026-11-30"));
        assert_eq!(day("November 20").as_deref(), Some("2026-11-20"));
        assert_eq!(day("Nov 20th").as_deref(), Some("2026-11-20"));
        assert_eq!(day("the 3rd of December").as_deref(), Some("2026-12-03"));
        assert_eq!(
            day("20 Sept").as_deref(),
            Some("2027-09-20"),
            "past: next year"
        );
        assert_eq!(day("2026-12-01").as_deref(), Some("2026-12-01"));
    }

    #[test]
    fn may_the_verb_is_not_may_the_month() {
        assert_eq!(day("I may get to it Friday").as_deref(), Some("2026-11-13"));
        assert_eq!(day("May 4").as_deref(), Some("2027-05-04"));
    }

    #[test]
    fn times_of_day_come_along() {
        assert_eq!(time("by 3pm").as_deref(), Some("15:00"));
        assert_eq!(day("by 3pm").as_deref(), Some("2026-11-12"));
        assert_eq!(time("Friday at 9:30 am").as_deref(), Some("09:30"));
        assert_eq!(day("Friday at 9:30 am").as_deref(), Some("2026-11-13"));
        assert_eq!(time("tomorrow 17:00").as_deref(), Some("17:00"));
        assert_eq!(time("today at noon").as_deref(), Some("12:00"));
        assert_eq!(time("12am").as_deref(), Some("00:00"));
        assert_eq!(time("Friday").as_deref(), None);
    }

    #[test]
    fn what_names_no_day_stays_unresolved() {
        for said in [
            "next week",
            "soon",
            "ASAP",
            "Q1",
            "later",
            "when possible",
            "",
            "May",
        ] {
            assert_eq!(resolve(said, thu()), None, "{said:?}");
        }
    }
}

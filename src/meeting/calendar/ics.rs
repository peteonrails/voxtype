//! Events from an iCalendar (ICS) feed: the private iCal link that Google
//! Calendar, Outlook, iCloud and Fastmail all offer.
//!
//! Timed events are expanded into the occurrences inside a window around
//! now: RRULE recurrence with EXDATE exclusions, RECURRENCE-ID overrides
//! (moved or retitled single occurrences) and STATUS:CANCELLED, with times
//! in their TZID zones. All-day events are not meetings and are skipped.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use icalendar::parser::{read_calendar, unfold, Component, Property};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// One occurrence of a timed event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// A video call link, from URL, LOCATION or a conference property
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Hosts whose links are video calls.
const MEETING_HOSTS: &[&str] = &[
    "zoom.us",
    "zoom.com",
    "meet.google.com",
    "teams.microsoft.com",
    "teams.live.com",
    "webex.com",
    "whereby.com",
    "meet.jit.si",
    "gotomeeting.com",
    "chime.aws",
    "bluejeans.com",
    "around.co",
    "app.slack.com",
    "discord.gg",
];

/// The video call link in `text`, if it holds one.
pub fn meeting_link(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | ',' | ';'))
        .filter(|w| w.starts_with("https://"))
        .map(|w| w.trim_end_matches(['.', ')', ']']))
        .find(|w| {
            let host = w["https://".len()..]
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("")
                .to_lowercase();
            MEETING_HOSTS
                .iter()
                .any(|h| host == *h || host.ends_with(&format!(".{h}")))
        })
        .map(str::to_string)
}

/// Outlook names zones the Windows way; the common ones, in IANA terms.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("Canada Central Standard Time", "America/Regina"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Argentina Standard Time", "America/Argentina/Buenos_Aires"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("GTB Standard Time", "Europe/Bucharest"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("E. Australia Standard Time", "Australia/Brisbane"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("UTC", "UTC"),
    ("Coordinated Universal Time", "UTC"),
];

/// The zone a TZID names: IANA, the Windows names Outlook uses, or the
/// local zone when it is neither.
fn zone(tzid: Option<&str>) -> rrule::Tz {
    let Some(tzid) = tzid
        .map(|t| t.trim().trim_matches('"'))
        .filter(|t| !t.is_empty())
    else {
        return rrule::Tz::LOCAL;
    };
    let iana = WINDOWS_ZONES
        .iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(tzid))
        .map(|(_, i)| *i)
        .unwrap_or(tzid);
    match chrono_tz::Tz::from_str(iana) {
        Ok(tz) => rrule::Tz::Tz(tz),
        Err(_) => {
            tracing::debug!("Unknown calendar time zone {:?}; using local time", tzid);
            rrule::Tz::LOCAL
        }
    }
}

/// A DTSTART-like value.
#[derive(Debug, Clone, Copy)]
enum When {
    /// A whole day: an all-day event
    Day,
    At(DateTime<rrule::Tz>),
}

fn param<'a>(p: &'a Property, key: &str) -> Option<&'a str> {
    p.params
        .iter()
        .find(|x| x.key.as_str().eq_ignore_ascii_case(key))
        .and_then(|x| x.val.as_ref())
        .map(|v| v.as_str())
}

fn prop<'a>(c: &'a Component, name: &str) -> Option<&'a Property<'a>> {
    c.properties
        .iter()
        .find(|p| p.name.as_str().eq_ignore_ascii_case(name))
}

fn text(c: &Component, name: &str) -> Option<String> {
    prop(c, name)
        .map(|p| unescape(p.val.as_str()))
        .filter(|s| !s.trim().is_empty())
}

/// RFC 5545 text escapes.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out.trim().to_string()
}

/// One date-time value, in the zone its TZID (or Z) gives.
fn parse_value(value: &str, tzid: Option<&str>, date_only: bool) -> Option<When> {
    let value = value.trim();
    if date_only || (value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit())) {
        NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some(When::Day);
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let naive = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some(When::At(
            Utc.from_utc_datetime(&naive).with_timezone(&rrule::Tz::UTC),
        ));
    }
    let naive = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    let tz = zone(tzid);
    // A time a DST change skips does not exist; take the later reading.
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(naive + Duration::hours(1)))
                .earliest()
        })
        .map(When::At)
}

fn when(p: &Property) -> Option<When> {
    let date_only = param(p, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
    parse_value(p.val.as_str(), param(p, "TZID"), date_only)
}

/// An ISO 8601 duration as DURATION gives it: P1W, PT1H30M, P1DT2H.
fn duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (unit, in_time) {
                    ('W', false) => Duration::weeks(n),
                    ('D', false) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    Some(if neg { -total } else { total })
}

/// A VEVENT as far as meetings care.
struct Vevent {
    uid: String,
    title: String,
    start: DateTime<rrule::Tz>,
    length: Duration,
    url: Option<String>,
    cancelled: bool,
    rrule: Option<String>,
    exdates: Vec<DateTime<rrule::Tz>>,
    rdates: Vec<DateTime<rrule::Tz>>,
    recurrence_id: Option<DateTime<Utc>>,
}

/// `None` for an all-day event, or one without a start or a title.
fn vevent(c: &Component) -> Option<Vevent> {
    let When::At(start) = when(prop(c, "DTSTART")?)? else {
        return None;
    };
    let end = prop(c, "DTEND").and_then(when);
    let length = match end {
        Some(When::At(end)) => end.with_timezone(&Utc) - start.with_timezone(&Utc),
        Some(When::Day) => return None,
        None => prop(c, "DURATION")
            .and_then(|p| duration(p.val.as_str()))
            .unwrap_or_else(Duration::zero),
    };
    let times = |name: &str| -> Vec<DateTime<rrule::Tz>> {
        c.properties
            .iter()
            .filter(|p| p.name.as_str().eq_ignore_ascii_case(name))
            .flat_map(|p| {
                let tzid = param(p, "TZID");
                let date_only = param(p, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
                p.val
                    .as_str()
                    .split(',')
                    .filter_map(|v| match parse_value(v, tzid, date_only)? {
                        When::At(t) => Some(t),
                        When::Day => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    let url = [
        "URL",
        "X-GOOGLE-CONFERENCE",
        "CONFERENCE",
        "LOCATION",
        "DESCRIPTION",
    ]
    .iter()
    .filter_map(|name| text(c, name))
    .find_map(|t| meeting_link(&t));
    Some(Vevent {
        uid: text(c, "UID").unwrap_or_default(),
        title: text(c, "SUMMARY").unwrap_or_default(),
        start,
        length: length.max(Duration::zero()),
        url,
        cancelled: text(c, "STATUS").is_some_and(|s| s.eq_ignore_ascii_case("CANCELLED")),
        rrule: prop(c, "RRULE").map(|p| p.val.as_str().trim().to_string()),
        exdates: times("EXDATE"),
        rdates: times("RDATE"),
        recurrence_id: prop(c, "RECURRENCE-ID")
            .and_then(when)
            .and_then(|w| match w {
                When::At(t) => Some(t.with_timezone(&Utc)),
                When::Day => None,
            }),
    })
}

/// The starts of `ev` from `from` to `to`.
fn occurrences(ev: &Vevent, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let Some(rule) = &ev.rrule else {
        return vec![ev.start.with_timezone(&Utc)];
    };
    let parsed = rrule::RRule::from_str(rule).and_then(|r| r.validate(ev.start));
    let rule = match parsed {
        Ok(rule) => rule,
        Err(e) => {
            tracing::debug!(
                "Skipping a calendar event with an RRULE voxtype cannot read: {}",
                e
            );
            return vec![ev.start.with_timezone(&Utc)];
        }
    };
    let tz = ev.start.timezone();
    let mut set = rrule::RRuleSet::new(ev.start)
        .rrule(rule)
        .after(from.with_timezone(&tz))
        .before(to.with_timezone(&tz));
    for ex in &ev.exdates {
        set = set.exdate(*ex);
    }
    for rd in &ev.rdates {
        set = set.rdate(*rd);
    }
    set.all(1000)
        .dates
        .into_iter()
        .map(|d| d.with_timezone(&Utc))
        .collect()
}

/// Every timed occurrence in `ics` overlapping `from`..`to`, earliest first.
/// Errors only when the text is not iCalendar at all.
pub fn events_between(
    ics: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<Event>, String> {
    if !ics
        .trim_start_matches('\u{feff}')
        .trim_start()
        .get(..15)
        .is_some_and(|head| head.eq_ignore_ascii_case("BEGIN:VCALENDAR"))
    {
        return Err("not an iCalendar feed (it does not start with BEGIN:VCALENDAR)".into());
    }
    let unfolded = unfold(ics);
    let calendar = read_calendar(&unfolded).map_err(|e| {
        // The parser quotes the input; keep only its first line.
        format!(
            "not an iCalendar feed ({})",
            e.lines().next().unwrap_or("parse error")
        )
    })?;
    let vevents: Vec<Vevent> = calendar
        .components
        .iter()
        .filter(|c| c.name.as_str().eq_ignore_ascii_case("VEVENT"))
        .filter_map(vevent)
        .collect();

    // Single occurrences moved, retitled or cancelled, by series and the
    // start they replace.
    let mut overrides: HashMap<&str, Vec<&Vevent>> = HashMap::new();
    for ev in vevents.iter().filter(|e| e.recurrence_id.is_some()) {
        overrides.entry(ev.uid.as_str()).or_default().push(ev);
    }

    let mut out = Vec::new();
    let mut add = |ev: &Vevent, start: DateTime<Utc>| {
        let end = start + ev.length;
        if ev.cancelled || ev.title.trim().is_empty() || end < from || start > to {
            return;
        }
        out.push(Event {
            title: ev.title.clone(),
            start,
            end,
            url: ev.url.clone(),
        });
    };
    for ev in vevents.iter().filter(|e| e.recurrence_id.is_none()) {
        let replaced: Vec<DateTime<Utc>> = overrides
            .get(ev.uid.as_str())
            .map(|o| o.iter().filter_map(|x| x.recurrence_id).collect())
            .unwrap_or_default();
        // An occurrence that started before `from` may still be running.
        for start in occurrences(ev, from - ev.length, to) {
            if !replaced.contains(&start) {
                add(ev, start);
            }
        }
    }
    for ev in vevents.iter().filter(|e| e.recurrence_id.is_some()) {
        add(ev, ev.start.with_timezone(&Utc));
    }
    out.sort_by(|a, b| (a.start, &a.title).cmp(&(b.start, &b.title)));
    Ok(out)
}

/// The event a meeting starting `now` is: one in progress, or starting
/// within `window_mins` either way; among several, the one whose start is
/// nearest `now`.
pub fn pick(events: &[Event], now: DateTime<Utc>, window_mins: u32) -> Option<&Event> {
    let window = Duration::minutes(window_mins as i64);
    events
        .iter()
        .filter(|e| (e.start <= now && now < e.end) || (e.start - now).abs() <= window)
        .min_by_key(|e| (e.start - now).abs())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A calendar as Google exports it: a weekly series with a moved and a
    /// cancelled occurrence and an EXDATE, an event in New York time, an
    /// all-day event, a cancelled one, and an Outlook-style zone.
    pub(crate) const FIXTURE: &str = include_str!("../../../tests/fixtures/calendar/fixture.ics");

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn titles(events: &[Event]) -> Vec<(String, String)> {
        events
            .iter()
            .map(|e| {
                (
                    e.start.format("%Y-%m-%dT%H:%MZ").to_string(),
                    e.title.clone(),
                )
            })
            .collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn a_weekly_series_with_a_moved_a_cancelled_and_an_excluded_week() {
        let all = events_between(
            FIXTURE,
            utc("2026-10-28T00:00:00Z"),
            utc("2026-12-04T00:00:00Z"),
        )
        .unwrap();
        let series: Vec<(String, String)> = titles(&all)
            .into_iter()
            .filter(|(_, t)| t.starts_with("Pete"))
            .collect();
        // London is on BST until 25 Oct, so 14:00 is 14:00Z from then on.
        assert_eq!(
            series,
            pairs(&[
                ("2026-10-29T14:00Z", "Pete <> Chad Bi-Weekly"),
                ("2026-11-05T15:00Z", "Pete <> Chad (moved)"),
                ("2026-11-12T14:00Z", "Pete <> Chad Bi-Weekly"),
                // 19 Nov excluded, 26 Nov cancelled.
                ("2026-12-03T14:00Z", "Pete <> Chad Bi-Weekly"),
            ])
        );
        assert_eq!(all[0].url.as_deref(), Some("https://zoom.us/j/111?pwd=abc"));
        assert_eq!(all[0].end - all[0].start, Duration::minutes(30));
    }

    #[test]
    fn zones_all_day_and_cancelled() {
        let day = events_between(
            FIXTURE,
            utc("2026-11-12T00:00:00Z"),
            utc("2026-11-13T00:00:00Z"),
        )
        .unwrap();
        assert_eq!(
            titles(&day),
            pairs(&[
                ("2026-11-12T14:00Z", "Design review, with NY"),
                ("2026-11-12T14:00Z", "Pete <> Chad Bi-Weekly"),
                ("2026-11-12T18:00Z", "Outlook planning"),
            ])
        );
        let ny = day.iter().find(|e| e.title.starts_with("Design")).unwrap();
        assert_eq!(
            ny.url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        let outlook = day.iter().find(|e| e.title.starts_with("Outlook")).unwrap();
        assert_eq!(outlook.end - outlook.start, Duration::minutes(45));
        assert!(outlook
            .url
            .as_deref()
            .unwrap()
            .starts_with("https://teams.microsoft.com/"));
        assert!(day
            .iter()
            .all(|e| e.title != "Company holiday" && e.title != "Cancelled sync"));
    }

    #[test]
    fn the_nearest_event_running_or_within_the_window_is_picked() {
        let day = events_between(
            FIXTURE,
            utc("2026-11-12T00:00:00Z"),
            utc("2026-11-13T00:00:00Z"),
        )
        .unwrap();
        // Two start at 14:00Z; at 13:55 both are 5 min away, earliest first.
        let at = |s: &str, w: u32| pick(&day, utc(s), w).map(|e| e.title.clone());
        assert_eq!(
            at("2026-11-12T13:55:00Z", 10).as_deref(),
            Some("Design review, with NY")
        );
        // In progress, 40 minutes in: the hour-long review is still on.
        assert_eq!(
            at("2026-11-12T14:40:00Z", 10).as_deref(),
            Some("Design review, with NY")
        );
        // Nothing within the window.
        assert_eq!(at("2026-11-12T16:00:00Z", 10), None);
        assert_eq!(at("2026-11-12T17:45:00Z", 10), None);
        assert_eq!(
            at("2026-11-12T17:45:00Z", 15).as_deref(),
            Some("Outlook planning")
        );
        assert_eq!(
            at("2026-11-12T18:20:00Z", 0).as_deref(),
            Some("Outlook planning")
        );
    }

    #[test]
    fn what_is_not_a_calendar_is_an_error_and_odd_values_are_skipped() {
        assert!(events_between("<html>nope</html>", Utc::now(), Utc::now()).is_err());
        let odd = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:garbage\r\nSUMMARY:x\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert_eq!(events_between(odd, Utc::now(), Utc::now()).unwrap(), vec![]);
    }

    #[test]
    fn meeting_links_and_durations() {
        assert_eq!(
            meeting_link("Room 4, or https://acme.zoom.us/j/1?pwd=x."),
            Some("https://acme.zoom.us/j/1?pwd=x".into())
        );
        assert_eq!(meeting_link("https://example.com/agenda"), None);
        assert_eq!(meeting_link("Board room"), None);
        assert_eq!(duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(duration("P1DT2H"), Some(Duration::hours(26)));
        assert_eq!(duration("P1W"), Some(Duration::weeks(1)));
        assert_eq!(duration("1H"), None);
    }
}

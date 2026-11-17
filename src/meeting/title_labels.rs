//! Speaker names from the title of a two-person meeting.
//!
//! "Pete <> Andrew standup" says who is in the room: the side that is the
//! user names the microphone, the other side names every remote voice. In a
//! 1:1 every remote voice is that one person, so this also absorbs a
//! diarizer that splits one remote speaker into several IDs.
//!
//! The parser is deliberately strict. A wrong name on a transcript is worse
//! than no name, so anything it is not sure of (three names, lowercase
//! words, team names, two sides that could both be the user) yields
//! nothing.

use std::collections::{HashMap, HashSet};

use super::data::{AudioSource, TranscriptSegment};
use super::storage::{MeetingStorage, StorageError};
use super::MeetingId;

/// Words between the two names: "Pete <> Andrew", "Pete / Chris",
/// "Pete x Chris", "Pete & Chris", "Pete and Chris", "Chris - Pete".
const SEPARATORS: &[&str] = &["<>", "<->", "/", "&", "x", "and", "-", "–", "—"];

/// Words a meeting title wraps the names in. Matched ignoring case and a
/// trailing colon, and only before the first name or after the second.
const NOISE: &[&str] = &[
    "1:1",
    "1-1",
    "1on1",
    "1-on-1",
    "one-on-one",
    "o3",
    "one-to-one",
    "1-to-1",
    "1to1",
    "1x1",
    "1:1s",
    "one",
    "on",
    "to",
    "weekly",
    "biweekly",
    "bi-weekly",
    "bi",
    "semi-weekly",
    "fortnightly",
    "daily",
    "monthly",
    "bi-monthly",
    "bimonthly",
    "semi-monthly",
    "quarterly",
    "annual",
    "yearly",
    "recurring",
    "standing",
    "standup",
    "stand-up",
    "sync",
    "sync-up",
    "syncup",
    "check-in",
    "checkin",
    "check",
    "in",
    "catch-up",
    "catchup",
    "catch",
    "up",
    "touch",
    "base",
    "touchbase",
    "touch-base",
    "review",
    "planning",
    "retro",
    "kickoff",
    "prep",
    "debrief",
    "session",
    "update",
    "updates",
    "discussion",
    "brainstorm",
    "interview",
    "chat",
    "meeting",
    "mtg",
    "call",
    "huddle",
    "coffee",
    "intro",
];

/// Capitalised words that are not a person's name: teams, products,
/// meeting kinds. A side containing one is not a name.
const NOT_NAMES: &[&str] = &[
    "all",
    "hands",
    "team",
    "teams",
    "product",
    "design",
    "engineering",
    "eng",
    "sales",
    "marketing",
    "support",
    "ops",
    "finance",
    "legal",
    "leadership",
    "exec",
    "staff",
    "board",
    "company",
    "customer",
    "client",
    "partner",
    "vendor",
    "frontend",
    "backend",
    "platform",
    "infra",
    "data",
    "research",
    "zoom",
    "meet",
    "google",
    "microsoft",
    "slack",
    "webex",
    "review",
    "planning",
    "interview",
    "demo",
    "retro",
    "kickoff",
    "roadmap",
    "strategy",
    "onboarding",
    "update",
    "discussion",
    "session",
    "project",
    "prep",
    "brainstorm",
    "budget",
    "hiring",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "the",
    "my",
    "our",
    "your",
    "q1",
    "q2",
    "q3",
    "q4",
];

/// The two names in a two-person meeting title, in title order, or `None`
/// when the title is not clearly that.
pub fn parse_pair(title: &str) -> Option<(String, String)> {
    let title = strip_noise_brackets(title)?;
    if title.contains([',', ';', '(', ')', '[', ']', '+', '@', '#']) {
        return None;
    }
    // Separators written without spaces: "Pete/Chris", "Pete<>Andrew".
    let spaced = title
        .replace("<->", " <-> ")
        .replace("<>", " <> ")
        .replace('/', " / ")
        .replace('&', " & ")
        .replace('–', " – ")
        .replace('—', " — ")
        // A colon ending a word sets off what follows ("Chad: Weekly"), but
        // "1:1" keeps its own.
        .replace(": ", " : ");
    let mut tokens: Vec<&str> = spaced.split_whitespace().collect();

    // Meeting words and the dashes or bars that set them off, at either end
    // of the title: "Weekly: Pete / Chad", "Pete <> Chad - Bi-Weekly",
    // "Pete <> Chad | Weekly Sync". Only then does a dash count as the one
    // separator between the names.
    let edge = |t: &str| is_noise(t) || matches!(t, "-" | "–" | "—" | "|" | "·" | "•" | ":");
    while tokens.last().is_some_and(|t| edge(t)) {
        tokens.pop();
    }
    while tokens.first().is_some_and(|t| edge(t)) {
        tokens.remove(0);
    }

    let mut separators = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| is_separator(t))
        .map(|(i, _)| i);
    let at = separators.next()?;
    if separators.next().is_some() {
        return None;
    }

    let mut left = &tokens[..at];
    let mut right = &tokens[at + 1..];
    while let Some((first, rest)) = left.split_first() {
        if !is_noise(first) {
            break;
        }
        left = rest;
    }
    while let Some((last, rest)) = right.split_last() {
        if !is_noise(last) {
            break;
        }
        right = rest;
    }

    let a = name(left)?;
    let b = name(right)?;
    if a.eq_ignore_ascii_case(&b) {
        return None;
    }
    Some((a, b))
}

/// The title with any bracketed meeting words removed ("Pete <> Chad
/// (Bi-Weekly)"), or `None` when brackets hold anything else or do not
/// pair up.
fn strip_noise_brackets(title: &str) -> Option<String> {
    let mut out = String::with_capacity(title.len());
    let mut rest = title;
    while let Some(open) = rest.find(['(', '[']) {
        let close_char = if rest[open..].starts_with('(') {
            ')'
        } else {
            ']'
        };
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find(close_char)?;
        let inner = &after[..close];
        if inner.contains(['(', '[', ')', ']'])
            || !inner
                .split(|c: char| c.is_whitespace() || c == '/')
                .filter(|w| !w.is_empty())
                .all(is_noise)
        {
            return None;
        }
        out.push(' ');
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn is_separator(token: &str) -> bool {
    // "And" is a word, but "X" may be an initial: only lowercase x counts.
    token == "x" || token.eq_ignore_ascii_case("and") || SEPARATORS.contains(&token)
}

fn is_noise(token: &str) -> bool {
    let word = token.trim_end_matches(':').to_lowercase();
    NOISE.contains(&word.as_str())
}

/// One or two capitalised name words, joined by a space.
fn name(words: &[&str]) -> Option<String> {
    if words.is_empty() || words.len() > 2 {
        return None;
    }
    words
        .iter()
        .all(|w| is_name_word(w))
        .then(|| words.join(" "))
}

fn is_name_word(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let rest: Vec<char> = chars.collect();
    first.is_uppercase()
        && !rest.is_empty()
        // "UX", "QBR": acronyms, not names.
        && !rest.iter().all(|c| c.is_uppercase())
        && rest
            .iter()
            .all(|c| c.is_alphabetic() || matches!(c, '\'' | '’' | '-'))
        && !NOT_NAMES.contains(&word.to_lowercase().as_str())
}

/// Who the user is, for telling their side of a title from the other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    /// The names the user goes by, the one to label with first.
    names: Vec<String>,
    /// The login name, used only when no name is known, and only ever as
    /// a first name: "pete" matches "Pete" in a title, never "Pete Smith"
    /// over "Pete Jones".
    login: Option<String>,
}

impl Me {
    /// The user as `meeting.my_name` names them, or else as the account's
    /// full name (GECOS) does, or else, when the account has no full name,
    /// by its login name as a first name. `None` when none of these is
    /// known or usable.
    pub fn resolve(my_name: Option<&str>) -> Option<Self> {
        let names: Vec<String> = my_name
            .map(str::to_string)
            .into_iter()
            .chain(account_full_name())
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty())
            .collect();
        if !names.is_empty() {
            return Some(Self { names, login: None });
        }
        let login = account_login().filter(|l| usable_login(l))?;
        Some(Self {
            names: Vec::new(),
            login: Some(login.to_lowercase()),
        })
    }

    /// A user known by these names, the first being what to label with.
    pub fn named(names: &[&str]) -> Self {
        Self {
            names: names.iter().map(|n| n.to_string()).collect(),
            login: None,
        }
    }

    /// A user known only by a login name.
    #[cfg(test)]
    pub(crate) fn logged_in_as(login: &str) -> Self {
        Self {
            names: Vec::new(),
            login: Some(login.to_lowercase()),
        }
    }

    /// How closely a title's name is this user: 2 for the whole name,
    /// 1 for the first name alone, 0 for someone else.
    fn match_score(&self, side: &str) -> u8 {
        let first = |s: &str| s.split_whitespace().next().unwrap_or("").to_lowercase();
        self.names
            .iter()
            .map(|n| {
                if n.eq_ignore_ascii_case(side) {
                    2
                } else if first(n) == first(side) {
                    1
                } else {
                    0
                }
            })
            .max()
            .unwrap_or(0)
            .max(u8::from(
                self.login.as_deref().is_some_and(|l| first(side) == l),
            ))
    }
}

/// Login names that are not a person's first name.
const NOT_FIRST_NAMES: &[&str] = &[
    "root",
    "admin",
    "administrator",
    "user",
    "guest",
    "test",
    "demo",
    "ubuntu",
    "debian",
    "arch",
    "fedora",
    "nixos",
    "alarm",
    "pi",
    "deck",
    "liveuser",
    "live",
    "vagrant",
    "docker",
    "builder",
    "build",
    "dev",
    "home",
    "me",
    "owner",
];

/// Whether a login name can stand for a first name: letters only, at least
/// two of them, and not a system or placeholder account.
fn usable_login(login: &str) -> bool {
    login.chars().count() >= 2
        && login.chars().all(char::is_alphabetic)
        && !NOT_FIRST_NAMES.contains(&login.to_lowercase().as_str())
}

#[cfg(test)]
thread_local! {
    /// The account name tests see in place of the real one.
    static TEST_ACCOUNT_NAME: std::cell::RefCell<Option<Option<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Make this thread's tests see `name` as the account's full name, so
/// they do not depend on who runs them.
#[cfg(test)]
pub(crate) fn set_account_name_for_test(name: Option<&str>) {
    TEST_ACCOUNT_NAME.with(|n| *n.borrow_mut() = Some(name.map(str::to_string)));
}

#[cfg(test)]
thread_local! {
    static TEST_LOGIN: std::cell::RefCell<Option<Option<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Make this thread's tests see `login` as the account's login name.
#[cfg(test)]
pub(crate) fn set_login_for_test(login: Option<&str>) {
    TEST_LOGIN.with(|n| *n.borrow_mut() = Some(login.map(str::to_string)));
}

/// The account's login name, from the account database, else $USER.
fn account_login() -> Option<String> {
    #[cfg(test)]
    if let Some(login) = TEST_LOGIN.with(|n| n.borrow().clone()) {
        return login;
    }
    passwd_field(|pwd| pwd.pw_name)
        .or_else(|| std::env::var("USER").ok())
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

/// One string field of the current user's passwd entry.
fn passwd_field(field: impl Fn(&libc::passwd) -> *mut libc::c_char) -> Option<String> {
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is to a live local sized as passed, and the
    // string read below points into `buf`, which outlives it.
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut pwd,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        )
    };
    let ptr = field(&pwd);
    if rc != 0 || result.is_null() || ptr.is_null() {
        return None;
    }
    Some(
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned(),
    )
}

/// The account's full name: the first comma-separated field of GECOS.
fn account_full_name() -> Option<String> {
    #[cfg(test)]
    if let Some(name) = TEST_ACCOUNT_NAME.with(|n| n.borrow().clone()) {
        return name;
    }
    let gecos = passwd_field(|pwd| pwd.pw_gecos)?;
    let full = gecos.split(',').next().unwrap_or("").trim().to_string();
    (!full.is_empty()).then_some(full)
}

/// The names a title gives the two sides of a meeting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleLabels {
    /// For the microphone's `You`
    pub you: String,
    /// For every remote voice
    pub other: String,
}

/// The labels `title` gives a meeting `me` is in: `None` unless it names
/// exactly two people and exactly one of them is clearly the user.
pub fn labels_for(title: &str, me: &Me) -> Option<TitleLabels> {
    let (a, b) = parse_pair(title)?;
    let (score_a, score_b) = (me.match_score(&a), me.match_score(&b));
    let (mine, other) = match score_a.cmp(&score_b) {
        std::cmp::Ordering::Greater => (a, b),
        std::cmp::Ordering::Less => (b, a),
        std::cmp::Ordering::Equal => return None,
    };
    Some(TitleLabels {
        // Known only by a login name: the title spells the name better.
        you: me.names.first().cloned().unwrap_or(mine),
        other,
    })
}

/// The speakers among `segments` the title names, as `(key, label)`:
/// the microphone's `You`, and each speaker heard only on the loopback.
/// A speaker heard on both tracks, or on a mixed one, is left alone.
pub fn assignments<'a>(
    labels: &TitleLabels,
    segments: impl IntoIterator<Item = &'a TranscriptSegment>,
) -> Vec<(String, String)> {
    let mut sources: HashMap<String, HashSet<AudioSource>> = HashMap::new();
    for segment in segments {
        sources
            .entry(segment.speaker_key())
            .or_default()
            .insert(segment.source);
    }
    let mut out: Vec<(String, String)> = sources
        .into_iter()
        .filter_map(|(key, sources)| {
            let only = |s: AudioSource| sources.len() == 1 && sources.contains(&s);
            if key == "You" && only(AudioSource::Microphone) {
                Some((key, labels.you.clone()))
            } else if key != "You" && only(AudioSource::Loopback) {
                Some((key, labels.other.clone()))
            } else {
                None
            }
        })
        .collect();
    out.sort();
    out
}

/// Name the speakers of a stored meeting from its title, leaving every
/// speaker that already has a name alone. Returns the speakers named.
pub fn label_stored(
    storage: &MeetingStorage,
    meeting_id: &MeetingId,
    me: &Me,
) -> Result<Vec<String>, StorageError> {
    let Some(metadata) = storage.get_meeting(meeting_id)? else {
        return Err(StorageError::NotFound(meeting_id.to_string()));
    };
    let Some(labels) = metadata.title.as_deref().and_then(|t| labels_for(t, me)) else {
        return Ok(Vec::new());
    };
    let transcript = storage.load_transcript_all(meeting_id)?;
    let mut named = Vec::new();
    for (key, label) in assignments(&labels, &transcript.segments) {
        if storage.label_speaker_if_unlabeled(meeting_id, &key, &label)? {
            named.push(key);
        }
    }
    if !named.is_empty() {
        storage.save_transcript(meeting_id, &transcript)?;
        tracing::info!(
            "Named {} speaker(s) from the title {:?}",
            named.len(),
            metadata.title.unwrap_or_default()
        );
    }
    Ok(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(title: &str) -> Option<(String, String)> {
        parse_pair(title)
    }

    fn p(a: &str, b: &str) -> Option<(String, String)> {
        Some((a.to_string(), b.to_string()))
    }

    #[test]
    fn two_person_titles_parse() {
        assert_eq!(pair("Pete <> Andrew standup"), p("Pete", "Andrew"));
        assert_eq!(pair("Andrew <> Pete Standup"), p("Andrew", "Pete"));
        assert_eq!(pair("Pete / Chris"), p("Pete", "Chris"));
        assert_eq!(pair("Pete/Chris"), p("Pete", "Chris"));
        assert_eq!(pair("Pete x Chris"), p("Pete", "Chris"));
        assert_eq!(pair("Pete & Chris"), p("Pete", "Chris"));
        assert_eq!(pair("Pete and Chris 1:1"), p("Pete", "Chris"));
        assert_eq!(pair("1:1 Pete/Chris"), p("Pete", "Chris"));
        assert_eq!(pair("1:1: Pete <> Chris"), p("Pete", "Chris"));
        assert_eq!(pair("Chris - Pete sync"), p("Chris", "Pete"));
        assert_eq!(pair("Chris – Pete"), p("Chris", "Pete"));
        assert_eq!(pair("Pete<>Andrew"), p("Pete", "Andrew"));
        assert_eq!(pair("Weekly Pete <> Andrew"), p("Pete", "Andrew"));
        assert_eq!(pair("Pete <> Andrew weekly check-in"), p("Pete", "Andrew"));
        assert_eq!(pair("Pete <> Andrew check in"), p("Pete", "Andrew"));
        assert_eq!(pair("Pete <> Andrew Catch Up"), p("Pete", "Andrew"));
        assert_eq!(
            pair("Pete Jackson <> Andrew Smith 1:1"),
            p("Pete Jackson", "Andrew Smith")
        );
        assert_eq!(
            pair("Mary-Jane O'Brien / Pete"),
            p("Mary-Jane O'Brien", "Pete")
        );
        assert_eq!(pair("Zoë & José"), p("Zoë", "José"));
    }

    /// Calendar titles are often title-cased, so meeting words count in any
    /// case, alone or several, before or after the names, bare or set off
    /// by brackets, a dash or a bar.
    #[test]
    fn meeting_words_never_become_part_of_a_name() {
        for title in [
            "Pete <> Chad Bi-Weekly",
            "Pete <> Chad bi-weekly",
            "Pete <> Chad BI-WEEKLY",
            "Pete <> Chad Biweekly",
            "Pete <> Chad Bi Weekly",
            "Pete <> Chad Weekly",
            "Pete <> Chad Fortnightly",
            "Pete <> Chad Monthly",
            "Pete <> Chad Quarterly",
            "Pete <> Chad Quarterly Review",
            "Pete <> Chad Daily",
            "Pete <> Chad Weekly Sync",
            "Pete <> Chad Sync-Up",
            "Pete <> Chad Sync Up",
            "Pete <> Chad Catch-Up",
            "Pete <> Chad Catch Up",
            "Pete <> Chad Check-In",
            "Pete <> Chad 1on1",
            "Pete <> Chad 1-on-1",
            "Pete <> Chad One-on-One",
            "Pete <> Chad One on One",
            "Pete <> Chad 1:1",
            "Pete <> Chad Standup",
            "Pete <> Chad Stand-Up",
            "Pete <> Chad (Bi-Weekly)",
            "Pete <> Chad [Weekly Sync]",
            "Pete <> Chad - Bi-Weekly",
            "Pete <> Chad | Weekly",
            "Pete <> Chad: Weekly",
            "1-on-1: Pete <> Chad",
            "Weekly - Pete / Chad",
            "Bi-Weekly Pete & Chad Sync",
            "Pete x Chad Recurring",
        ] {
            assert_eq!(pair(title), p("Pete", "Chad"), "{title:?}");
        }
        // Pete's own title, end to end.
        let labels = labels_for("Pete <> Chad Bi-Weekly", &Me::named(&["Pete Jackson"])).unwrap();
        assert_eq!(labels.other, "Chad");
        assert_eq!(pair("Chad Lee <> Pete Monthly"), p("Chad Lee", "Pete"));
    }

    #[test]
    fn anything_else_does_not() {
        for title in [
            "",
            "Zoom call",
            "Q4 planning",
            "Pete, Andrew, Sam sync",
            "Pete / Chris / Sam",
            "Pete & Chris and Sam",
            "Pete <> Andrew & Sam standup",
            "Standup",
            "Pete",
            "Pete sync",
            "Design / Eng sync",
            "Product <> Sales",
            "All Hands",
            "Q3 / Q4 planning",
            "Pete <> UX review",
            "Pete <> the team",
            "pete <> andrew",
            "Pete <> Andrew quick sync",
            "Pete <> Andrew Roadmap",
            "Pete <> Andrew: roadmap",
            "Pete <> Pete",
            "Pete <> Andrew Lee Smith",
            "Pete <> Chad (Roadmap)",
            "Pete <> Chad (weekly",
            "Pete <> Chad - Roadmap",
            "Pete <> Chad Bi-Weekly Roadmap",
            "Pete + Andrew",
            "Pete X Andrew",
            "Pete <> Andrew 2",
            "Meeting with Andrew",
            "1:1",
        ] {
            assert_eq!(parse_pair(title), None, "{title:?}");
        }
    }

    #[test]
    fn the_side_that_is_the_user_names_you() {
        let me = Me::named(&["Pete"]);
        let labels = labels_for("Pete <> Andrew standup", &me).unwrap();
        assert_eq!(labels.you, "Pete");
        assert_eq!(labels.other, "Andrew");
        // Either order, any case.
        let labels = labels_for("andrew <> pete", &me);
        assert_eq!(labels, None, "lowercase names never parse");
        let labels = labels_for("Andrew <> Pete Standup", &me).unwrap();
        assert_eq!(labels.other, "Andrew");
        let labels = labels_for("Andrew <> PETE", &me);
        assert_eq!(labels, None, "an all-caps word is an acronym");
    }

    #[test]
    fn a_first_name_matches_a_full_name_and_back() {
        let me = Me::named(&["Pete Jackson"]);
        let labels = labels_for("Pete / Chris", &me).unwrap();
        assert_eq!(labels.you, "Pete Jackson", "labelled with the user's name");
        assert_eq!(labels.other, "Chris");

        let me = Me::named(&["Pete"]);
        let labels = labels_for("Chris - Pete Jackson sync", &me).unwrap();
        assert_eq!(labels.other, "Chris");
    }

    #[test]
    fn my_name_wins_over_the_account_name() {
        let me = Me::named(&["Peter", "Pete Jackson"]);
        let labels = labels_for("Pete <> Andrew", &me).unwrap();
        assert_eq!(labels.you, "Peter");
        assert_eq!(labels.other, "Andrew");
    }

    #[test]
    fn neither_or_both_sides_being_the_user_names_nobody() {
        let me = Me::named(&["Pete Jackson"]);
        assert_eq!(labels_for("Andrew <> Chris", &me), None);
        // Both first names match and neither is the whole name.
        assert_eq!(labels_for("Pete Smith <> Pete Jones", &me), None);
        // The whole name beats a first name.
        let labels = labels_for("Pete Smith <> Pete Jackson", &me).unwrap();
        assert_eq!(labels.other, "Pete Smith");
    }

    #[test]
    fn resolve_prefers_my_name_then_the_account_name() {
        set_account_name_for_test(Some("Pete Jackson"));
        assert_eq!(
            Me::resolve(Some("  Peter  ")).unwrap().names,
            ["Peter", "Pete Jackson"]
        );
        assert_eq!(Me::resolve(Some("")).unwrap().names, ["Pete Jackson"]);
        assert_eq!(Me::resolve(None).unwrap().names, ["Pete Jackson"]);
        set_account_name_for_test(None);
        set_login_for_test(None);
        assert_eq!(Me::resolve(None), None, "nobody to match a title against");
        assert_eq!(Me::resolve(Some("Pete")).unwrap().names, ["Pete"]);
    }

    /// An account with an empty GECOS field, as on Pete's machine
    /// (`pete:x:1000:1000::...`): the login name stands in as a first name.
    #[test]
    fn with_no_full_name_the_login_name_places_you() {
        set_account_name_for_test(None);
        set_login_for_test(Some("pete"));
        let me = Me::resolve(None).unwrap();
        assert_eq!(me, Me::logged_in_as("pete"));
        let labels = labels_for("Pete <> Chad", &me).unwrap();
        assert_eq!(
            (labels.you.as_str(), labels.other.as_str()),
            ("Pete", "Chad")
        );
        let labels = labels_for("Chad / Pete Jackson weekly", &me).unwrap();
        assert_eq!(
            (labels.you.as_str(), labels.other.as_str()),
            ("Pete Jackson", "Chad")
        );
        // Only ever a first name: two Petes stay a tie.
        assert_eq!(labels_for("Pete Smith <> Pete Jones", &me), None);
        assert_eq!(labels_for("Andrew <> Chad", &me), None);

        // A full name, when there is one, wins over the login.
        set_account_name_for_test(Some("Peter Jackson"));
        let me = Me::resolve(None).unwrap();
        assert_eq!(me, Me::named(&["Peter Jackson"]));
        assert_eq!(labels_for("Pete <> Chad", &me), None, "Pete is not Peter");
        // And my_name wins over both.
        assert_eq!(Me::resolve(Some("Pete")).unwrap().names[0], "Pete");
    }

    /// The order the user is known by: `meeting.my_name`, then the
    /// account's full name, then the login name, which only stands in when
    /// neither name is known.
    #[test]
    fn my_name_then_full_name_then_login() {
        set_login_for_test(Some("pete"));
        set_account_name_for_test(None);
        let me = Me::resolve(Some("Peter")).unwrap();
        assert_eq!(me, Me::named(&["Peter"]), "my_name, and no login fallback");
        let labels = labels_for("Peter <> Chad", &me).unwrap();
        assert_eq!(labels.you, "Peter");

        set_account_name_for_test(Some("Pete Jackson"));
        assert_eq!(
            Me::resolve(Some("Peter")).unwrap(),
            Me::named(&["Peter", "Pete Jackson"])
        );
        assert_eq!(Me::resolve(None).unwrap(), Me::named(&["Pete Jackson"]));

        set_account_name_for_test(None);
        assert_eq!(Me::resolve(None).unwrap(), Me::logged_in_as("pete"));
        assert_eq!(Me::resolve(Some("  ")).unwrap(), Me::logged_in_as("pete"));
    }

    #[test]
    fn system_and_odd_logins_name_nobody() {
        set_account_name_for_test(None);
        for login in [
            "root", "user", "deck", "pi", "u1000", "12345", "p", "pete.j", "pete_j", "",
        ] {
            set_login_for_test(Some(login));
            assert_eq!(Me::resolve(None), None, "{login:?}");
        }
        set_login_for_test(Some("Zoë"));
        assert!(Me::resolve(None).is_some(), "letters beyond ASCII count");
    }

    #[test]
    fn the_real_account_name_is_read_without_failing() {
        // Whatever GECOS holds here, reading it must not panic, and a name
        // read is trimmed and not empty.
        if let Some(name) = account_full_name() {
            assert!(!name.is_empty() && name.trim() == name);
        }
    }

    fn seg(id: u32, source: AudioSource, speaker: &str) -> TranscriptSegment {
        let mut s = TranscriptSegment::new(id, 0, 1, "hi".into(), 0);
        s.source = source;
        s.speaker_id = Some(speaker.to_string());
        s
    }

    #[test]
    fn every_loopback_speaker_is_the_other_person() {
        let labels = TitleLabels {
            you: "Pete".into(),
            other: "Chris".into(),
        };
        let segments = [
            seg(0, AudioSource::Microphone, "You"),
            seg(1, AudioSource::Loopback, "Remote 1"),
            seg(2, AudioSource::Loopback, "Remote 2"),
            seg(3, AudioSource::Loopback, "Remote 5"),
            // Clustered on both tracks: could be either person.
            seg(4, AudioSource::Microphone, "SPEAKER_00"),
            seg(5, AudioSource::Loopback, "SPEAKER_00"),
            // A mixed recording has no sides.
            seg(6, AudioSource::Unknown, "SPEAKER_01"),
        ];
        let pairs = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert_eq!(
            assignments(&labels, &segments),
            pairs(&[
                ("Remote 1", "Chris"),
                ("Remote 2", "Chris"),
                ("Remote 5", "Chris"),
                ("You", "Pete"),
            ])
        );
    }

    fn stored(title: &str) -> (MeetingStorage, tempfile::TempDir, MeetingId) {
        use crate::meeting::data::{MeetingMetadata, Transcript};
        let dir = tempfile::TempDir::new().unwrap();
        let storage = MeetingStorage::open(crate::meeting::StorageConfig {
            storage_path: dir.path().to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap();
        let mut metadata = MeetingMetadata::new(Some(title.to_string()));
        metadata.storage_path = Some(storage.create_meeting(&metadata).unwrap());
        let mut t = Transcript::new();
        for s in [
            seg(0, AudioSource::Microphone, "You"),
            seg(1, AudioSource::Loopback, "Remote 1"),
            seg(2, AudioSource::Loopback, "Remote 2"),
        ] {
            t.add_segment(s);
        }
        storage.save_transcript(&metadata.id, &t).unwrap();
        (storage, dir, metadata.id)
    }

    fn shown(storage: &MeetingStorage, id: &MeetingId) -> Vec<String> {
        let t = storage.load_transcript(id).unwrap();
        t.segments.iter().map(|s| s.speaker_display()).collect()
    }

    #[test]
    fn a_stored_meeting_is_named_without_touching_hand_labels() {
        let (storage, _dir, id) = stored("Pete <> Chris sync");
        storage.set_speaker_label(&id, "Remote 1", "Sam").unwrap();
        let me = Me::named(&["Pete Jackson"]);

        let named = label_stored(&storage, &id, &me).unwrap();
        assert_eq!(named, ["Remote 2", "You"]);
        assert_eq!(shown(&storage, &id), ["Pete Jackson", "Sam", "Chris"]);
        // transcript.json carries the names for readers outside voxtype.
        let folder = storage.get_meeting_path(&id).unwrap();
        let json = std::fs::read_to_string(folder.join("transcript.json")).unwrap();
        assert!(json.contains("\"Chris\"") && json.contains("\"Pete Jackson\""));

        // Again: everyone already has a name.
        assert!(label_stored(&storage, &id, &me).unwrap().is_empty());
        assert_eq!(shown(&storage, &id), ["Pete Jackson", "Sam", "Chris"]);
    }

    #[test]
    fn a_title_that_does_not_qualify_names_nobody() {
        let me = Me::named(&["Pete Jackson"]);
        for title in ["Zoom call", "Andrew <> Chris", "Pete, Andrew, Sam sync"] {
            let (storage, _dir, id) = stored(title);
            assert!(label_stored(&storage, &id, &me).unwrap().is_empty());
            assert_eq!(shown(&storage, &id), ["You", "Remote 1", "Remote 2"]);
        }
    }
}

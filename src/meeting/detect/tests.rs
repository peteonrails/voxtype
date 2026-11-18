//! Decision tests over recorded `pw-dump` and `hyprctl clients -j` fixtures.
//!
//! The fixtures in `tests/fixtures/meeting_detect/` keep the real shape of
//! both documents: the idle graph (another dictation app and the
//! camera-effects virtual mic, both always open) was captured on Omarchy,
//! and each call app's stream is added in the form pipewire-pulse gives it.
//! Window titles are scrubbed to the app and the call-title shape.

use super::*;
use crate::config::MeetingDetectConfig;

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/meeting_detect")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
}

fn snap(pw: &str, clients: Option<&str>) -> Snapshot {
    Snapshot {
        streams: pipewire::parse_dump(&fixture(pw)).unwrap(),
        windows: clients.map(|c| hyprland::parse_clients(&fixture(c)).unwrap()),
    }
}

fn config() -> MeetingDetectConfig {
    MeetingDetectConfig::default()
}

const IDLE: Activity = Activity {
    meeting_active: false,
    dictating: false,
};

/// Run a snapshot past the debounce and return everything decided.
fn settle(detector: &mut Detector, s: &Snapshot, activity: Activity) -> Vec<Event> {
    let t0 = Instant::now();
    let mut events = detector.evaluate(s, activity, t0);
    events.extend(detector.evaluate(s, activity, t0 + Duration::from_secs(6)));
    events
}

fn prompts(events: &[Event]) -> Vec<&Call> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Prompt(c) => Some(c),
            _ => None,
        })
        .collect()
}

#[test]
fn zoom_prompts_with_the_meeting_window_title() {
    let mut d = Detector::new(config());
    let events = settle(
        &mut d,
        &snap("pw-zoom.json", Some("clients-zoom.json")),
        IDLE,
    );
    let p = prompts(&events);
    assert_eq!(p.len(), 1, "{events:?}");
    assert_eq!(p[0].app, "Zoom");
    assert_eq!(p[0].title, "Zoom call", "a placeholder window title");
}

#[test]
fn zoom_prompts_without_window_titles() {
    let mut d = Detector::new(config());
    let events = settle(&mut d, &snap("pw-zoom.json", None), IDLE);
    let p = prompts(&events);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].title, "Zoom call");
}

#[test]
fn meet_in_chromium_prompts_titled_from_the_tab() {
    let mut d = Detector::new(config());
    let events = settle(
        &mut d,
        &snap("pw-chromium-mic.json", Some("clients-meet-chromium.json")),
        IDLE,
    );
    let p = prompts(&events);
    assert_eq!(p.len(), 1, "{events:?}");
    assert_eq!(p[0].app, "Google Meet");
    assert_eq!(p[0].title, "Google Meet call", "a bare meeting code");
}

#[test]
fn meet_in_firefox_prompts_titled_from_the_tab() {
    let mut d = Detector::new(config());
    let events = settle(
        &mut d,
        &snap("pw-firefox-mic.json", Some("clients-meet-firefox.json")),
        IDLE,
    );
    let p = prompts(&events);
    assert_eq!(p.len(), 1, "{events:?}");
    assert_eq!(p[0].app, "Google Meet");
    assert_eq!(p[0].title, "Google Meet call", "a bare meeting code");
}

#[test]
fn teams_prompts() {
    let mut d = Detector::new(config());
    let events = settle(
        &mut d,
        &snap("pw-teams.json", Some("clients-teams.json")),
        IDLE,
    );
    let p = prompts(&events);
    assert_eq!(p.len(), 1, "{events:?}");
    assert_eq!(p[0].app, "Microsoft Teams");
    assert_eq!(p[0].title, "Weekly sync");
}

#[test]
fn a_plain_mic_recording_does_not_prompt() {
    let mut d = Detector::new(config());
    let s = snap("pw-plain-mic.json", Some("clients-browsing.json"));
    assert!(settle(&mut d, &s, IDLE).is_empty());
    let (_, notes) = find_calls(&s, d.config());
    assert!(notes
        .iter()
        .any(|n| n.0.contains("pw-record") && n.0.contains("not a call app")));
}

#[test]
fn a_browser_mic_without_a_call_window_does_not_prompt() {
    let mut d = Detector::new(config());
    let s = snap("pw-chromium-mic.json", Some("clients-browsing.json"));
    assert!(settle(&mut d, &s, IDLE).is_empty());
    let (_, notes) = find_calls(&s, d.config());
    assert!(
        notes
            .iter()
            .any(|n| n.0.contains("Chromium") && n.0.contains("no window title")),
        "{notes:?}"
    );
}

#[test]
fn a_browser_mic_off_hyprland_does_not_prompt() {
    let mut d = Detector::new(config());
    assert!(settle(&mut d, &snap("pw-chromium-mic.json", None), IDLE).is_empty());
}

#[test]
fn voxtypes_own_capture_never_counts() {
    let s = snap("pw-voxtype.json", Some("clients-browsing.json"));
    let (calls, notes) = find_calls(&s, &config());
    assert!(calls.is_empty());
    let own = notes
        .iter()
        .filter(|n| n.0.contains("voxtype's own capture"))
        .count();
    assert_eq!(own, 2, "{notes:?}");
    // Even a pattern broad enough to match it cannot make it a call.
    let mut broad = config();
    broad.apps = vec![CallAppPattern {
        name: "Anything".into(),
        streams: vec!["alsa".into(), "voxtype".into()],
        titles: vec![],
    }];
    assert!(find_calls(&s, &broad).0.is_empty());
}

#[test]
fn the_always_open_streams_on_omarchy_are_ignored() {
    let s = snap("pw-idle.json", None);
    let (calls, notes) = find_calls(&s, &config());
    assert!(calls.is_empty());
    assert!(notes.iter().any(|n| n.0.contains("another dictation app")));
    assert!(notes.iter().any(|n| n.0.contains("camera-effects")));
}

#[test]
fn a_brief_mic_open_is_debounced() {
    let mut d = Detector::new(config());
    let call = snap("pw-zoom.json", Some("clients-zoom.json"));
    let idle = snap("pw-idle.json", Some("clients-zoom.json"));
    let t0 = Instant::now();
    let first = d.evaluate(&call, IDLE, t0);
    assert!(
        matches!(first.as_slice(), [Event::Seen { .. }]),
        "{first:?}"
    );
    assert!(d
        .evaluate(&call, IDLE, t0 + Duration::from_secs(2))
        .is_empty());
    let gone = d.evaluate(&idle, IDLE, t0 + Duration::from_secs(3));
    assert!(matches!(gone.as_slice(), [Event::Brief { .. }]), "{gone:?}");
    assert!(!d.is_tracking());
    assert!(d
        .evaluate(&idle, IDLE, t0 + Duration::from_secs(10))
        .is_empty());
}

#[test]
fn one_call_prompts_once() {
    let mut d = Detector::new(config());
    let s = snap("pw-zoom.json", Some("clients-zoom.json"));
    let t0 = Instant::now();
    let mut all = Vec::new();
    for secs in 0..60 {
        all.extend(d.evaluate(&s, IDLE, t0 + Duration::from_secs(secs)));
    }
    assert_eq!(prompts(&all).len(), 1, "{all:?}");
}

#[test]
fn no_prompt_while_a_meeting_is_recording() {
    let mut d = Detector::new(config());
    let busy = Activity {
        meeting_active: true,
        dictating: false,
    };
    let events = settle(
        &mut d,
        &snap("pw-zoom.json", Some("clients-zoom.json")),
        busy,
    );
    assert!(prompts(&events).is_empty());
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::Held { why, .. } if why.contains("meeting"))));
}

#[test]
fn dictation_defers_the_prompt_until_it_ends() {
    let mut d = Detector::new(config());
    let s = snap("pw-zoom.json", Some("clients-zoom.json"));
    let dictating = Activity {
        meeting_active: false,
        dictating: true,
    };
    let t0 = Instant::now();
    d.evaluate(&s, dictating, t0);
    let held = d.evaluate(&s, dictating, t0 + Duration::from_secs(6));
    assert!(matches!(held.as_slice(), [Event::Held { .. }]), "{held:?}");
    assert!(d
        .evaluate(&s, dictating, t0 + Duration::from_secs(8))
        .is_empty());
    let after = d.evaluate(&s, IDLE, t0 + Duration::from_secs(10));
    assert_eq!(prompts(&after).len(), 1, "{after:?}");
}

#[test]
fn auto_record_apps_skip_the_prompt() {
    let mut cfg = config();
    cfg.auto_record_apps = vec!["zoom".into()];
    let mut d = Detector::new(cfg);
    let events = settle(
        &mut d,
        &snap("pw-zoom.json", Some("clients-zoom.json")),
        IDLE,
    );
    assert!(prompts(&events).is_empty());
    assert!(events
        .iter()
        .any(|e| matches!(e, Event::AutoRecord(c) if c.title == "Zoom call")));
}

#[test]
fn call_end_waits_out_a_short_gap_then_offers_stop() {
    let mut d = Detector::new(config());
    let call = snap("pw-zoom.json", Some("clients-zoom.json"));
    let idle = snap("pw-idle.json", Some("clients-zoom.json"));
    let recording = Activity {
        meeting_active: true,
        dictating: false,
    };
    let t0 = Instant::now();
    d.evaluate(&call, IDLE, t0);
    assert_eq!(
        prompts(&d.evaluate(&call, IDLE, t0 + Duration::from_secs(6))).len(),
        1
    );
    assert!(d
        .evaluate(&call, recording, t0 + Duration::from_secs(18))
        .is_empty());
    // Mic drops for two seconds (device switch) and comes back: same call.
    assert!(d
        .evaluate(&idle, recording, t0 + Duration::from_secs(20))
        .is_empty());
    assert!(d
        .evaluate(&call, recording, t0 + Duration::from_secs(22))
        .is_empty());
    // Gone for good.
    assert!(d
        .evaluate(&idle, recording, t0 + Duration::from_secs(24))
        .is_empty());
    let end = d.evaluate(&idle, recording, t0 + Duration::from_secs(30));
    assert!(
        matches!(
            end.as_slice(),
            [Event::Ended {
                offer_stop: true,
                ..
            }]
        ),
        "{end:?}"
    );
}

#[test]
fn call_end_without_a_recording_offers_nothing() {
    let mut d = Detector::new(config());
    let call = snap("pw-zoom.json", Some("clients-zoom.json"));
    let idle = snap("pw-idle.json", None);
    let t0 = Instant::now();
    d.evaluate(&call, IDLE, t0);
    d.evaluate(&call, IDLE, t0 + Duration::from_secs(6));
    d.evaluate(&idle, IDLE, t0 + Duration::from_secs(7));
    let end = d.evaluate(&idle, IDLE, t0 + Duration::from_secs(13));
    assert!(
        matches!(
            end.as_slice(),
            [Event::Ended {
                offer_stop: false,
                ..
            }]
        ),
        "{end:?}"
    );
}

#[test]
fn clean_title_strips_browser_suffix_and_unread_count() {
    assert_eq!(
        clean_title("Meet - abc-defg-hij - Chromium"),
        "Meet - abc-defg-hij"
    );
    assert_eq!(
        clean_title("(2) Meet - Standup — Mozilla Firefox"),
        "Meet - Standup"
    );
    assert_eq!(clean_title("Zoom Meeting"), "Zoom Meeting");
    assert_eq!(clean_title("(draft) notes"), "(draft) notes");
}

#[test]
fn read_activity_sees_meetings_and_dictation() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    assert_eq!(read_activity(dir.path(), Some(&state)), Activity::default());
    std::fs::write(dir.path().join("meeting_state"), "recording\nabc").unwrap();
    std::fs::write(&state, "recording").unwrap();
    assert_eq!(
        read_activity(dir.path(), Some(&state)),
        Activity {
            meeting_active: true,
            dictating: true
        }
    );
    std::fs::write(dir.path().join("meeting_state"), "idle").unwrap();
    std::fs::write(&state, "transcribing").unwrap();
    assert_eq!(read_activity(dir.path(), Some(&state)), Activity::default());
    std::fs::write(dir.path().join("meeting_start"), "title").unwrap();
    assert!(read_activity(dir.path(), None).meeting_active);
}

#[test]
fn report_explains_a_meet_call() {
    let lines = report(
        &snap("pw-chromium-mic.json", Some("clients-meet-chromium.json")),
        IDLE,
        &config(),
    );
    let text = lines.join("\n");
    assert!(text.contains("windows: 4 from Hyprland"), "{text}");
    assert!(text.contains("another dictation app"), "{text}");
    assert!(
        text.contains("would prompt: Record \"Google Meet call\"? (Google Meet)"),
        "{text}"
    );
}

#[test]
fn report_says_why_nothing_prompts() {
    let text = report(&snap("pw-idle.json", None), IDLE, &config()).join("\n");
    assert!(text.contains("no prompt: no call detected"), "{text}");
    assert!(text.contains("unavailable"), "{text}");
}

#[test]
fn placeholder_window_titles_become_call_titles() {
    for (app, title, want) in [
        (
            "Zoom",
            Some("Join from Zoom Workplace app - Zoom"),
            "Zoom call",
        ),
        ("Zoom", Some("Zoom Meeting"), "Zoom call"),
        ("Zoom", Some("Zoom Workplace"), "Zoom call"),
        ("Zoom", Some("Zoom"), "Zoom call"),
        ("Zoom", Some("Zoom Cloud Meetings"), "Zoom call"),
        ("Zoom", None, "Zoom call"),
        (
            "Zoom (web)",
            Some("Join from Zoom Workplace app - Zoom - Chromium"),
            "Zoom call",
        ),
        (
            "Google Meet",
            Some("Meet - abc-defg-hij - Chromium"),
            "Google Meet call",
        ),
        (
            "Google Meet",
            Some("Google Meet — Mozilla Firefox"),
            "Google Meet call",
        ),
        ("Google Meet", Some("Meet"), "Google Meet call"),
        ("Microsoft Teams", Some("Microsoft Teams"), "Teams call"),
        (
            "Microsoft Teams",
            Some("Meeting | Microsoft Teams"),
            "Teams call",
        ),
        (
            "Microsoft Teams (web)",
            Some("Calling | Microsoft Teams - Chromium"),
            "Teams call",
        ),
        ("Microsoft Teams", None, "Teams call"),
        ("Jitsi", None, "Jitsi call"),
    ] {
        assert_eq!(meeting_title(app, title), want, "{app} {title:?}");
    }
}

#[test]
fn real_meeting_names_are_kept() {
    for (app, title, want) in [
        (
            "Google Meet",
            Some("Meet - Weekly sync - Chromium"),
            "Weekly sync",
        ),
        (
            "Google Meet",
            Some("(2) Meet - Design review — Mozilla Firefox"),
            "Design review",
        ),
        (
            "Microsoft Teams",
            Some("Weekly sync | Microsoft Teams"),
            "Weekly sync",
        ),
        (
            "Microsoft Teams (web)",
            Some("Budget review | Microsoft Teams - Chromium"),
            "Budget review",
        ),
        (
            "Zoom",
            Some("Pete's Personal Meeting Room"),
            "Pete's Personal Meeting Room",
        ),
        (
            "Jitsi",
            Some("Jitsi Meet - standup"),
            "Jitsi Meet - standup",
        ),
    ] {
        assert_eq!(meeting_title(app, title), want, "{app} {title:?}");
    }
}

/// The announcement's targets: the call app's own capture streams, never
/// voxtype's, another dictation app's or the camera-effects mic.
#[test]
fn call_streams_are_the_call_apps_own_streams() {
    let s = snap("pw-zoom.json", Some("clients-zoom.json"));
    let found = call_streams(&s, &config());
    assert!(!found.is_empty(), "{:?}", s.streams);
    for (call, stream) in &found {
        assert_eq!(call.app, "Zoom");
        assert!(stream.ignore_reason().is_none(), "{stream:?}");
    }
    // The always-open streams in the idle graph are in the snapshot but
    // never targeted.
    assert!(s.streams.len() > found.len());
}

#[test]
fn a_browser_stream_is_a_target_only_with_a_call_window() {
    let with_call = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    let found = call_streams(&with_call, &config());
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0.app, "Google Meet");

    // The same mic stream while browsing, or off Hyprland, is no call, so
    // the announcement plays only locally rather than into a random tab.
    let browsing = snap("pw-chromium-mic.json", Some("clients-browsing.json"));
    assert!(call_streams(&browsing, &config()).is_empty());
    let no_titles = snap("pw-chromium-mic.json", None);
    assert!(call_streams(&no_titles, &config()).is_empty());
}

#[test]
fn no_call_means_no_targets() {
    for pw in ["pw-idle.json", "pw-voxtype.json", "pw-plain-mic.json"] {
        let s = snap(pw, Some("clients-browsing.json"));
        assert!(call_streams(&s, &config()).is_empty(), "{pw}");
    }
}

// What ends a call. The Zoom fixtures below have the shape of the calls
// that were cut off on 2026-11-17: Chromium's capture stream is "Chromium
// input", the web client runs in its own app window titled with the
// meeting's name, and the only title that confirms the call is the "Join
// from Zoom Workplace app" launcher tab in the main browser window.

const RECORDING: Activity = Activity {
    meeting_active: true,
    dictating: false,
};

fn secs(t0: Instant, s: u64) -> Instant {
    t0 + Duration::from_secs(s)
}

/// Feed `s` every two seconds, as the live loop does while a call is
/// tracked, from `from` up to but not including `to` (seconds after `t0`).
fn watch(
    d: &mut Detector,
    s: &Snapshot,
    activity: Activity,
    t0: Instant,
    from: u64,
    to: u64,
) -> Vec<Event> {
    (from..to)
        .step_by(2)
        .flat_map(|at| d.evaluate(s, activity, secs(t0, at)))
        .collect()
}

/// The events that would start, stop or offer anything.
fn acted(events: &[Event]) -> Vec<&Event> {
    events
        .iter()
        .filter(|e| !matches!(e, Event::Continues { .. }))
        .collect()
}

fn continues(events: &[Event]) -> Vec<Evidence> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Continues { evidence, .. } => Some(*evidence),
            _ => None,
        })
        .collect()
}

/// A Meet call in Chromium, prompted for and then recorded. Returns the
/// detector with the call settled at `t0 + 6`.
fn settled_meet_call(t0: Instant) -> Detector {
    let mut d = Detector::new(config());
    let call = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    d.evaluate(&call, IDLE, t0);
    let events = d.evaluate(&call, IDLE, secs(t0, 6));
    assert_eq!(prompts(&events).len(), 1, "{events:?}");
    d
}

#[test]
fn evidence_is_the_mic_and_for_a_browser_call_its_window() {
    let cfg = config();
    let app = |name: &str| cfg.apps.iter().find(|a| a.name == name).unwrap();
    for (name, pw, clients, want) in [
        (
            "Google Meet",
            "pw-chromium-mic.json",
            Some("clients-meet-chromium.json"),
            Evidence::Full,
        ),
        (
            "Google Meet",
            "pw-idle.json",
            Some("clients-meet-chromium.json"),
            Evidence::WindowOnly,
        ),
        (
            "Google Meet",
            "pw-chromium-mic.json",
            Some("clients-browsing.json"),
            Evidence::MicOnly,
        ),
        (
            "Google Meet",
            "pw-idle.json",
            Some("clients-browsing.json"),
            Evidence::Gone,
        ),
        (
            "Google Meet",
            "pw-chromium-mic.json",
            None,
            Evidence::WindowsUnknown { mic: true },
        ),
        (
            "Google Meet",
            "pw-idle.json",
            None,
            Evidence::WindowsUnknown { mic: false },
        ),
        // Any browser the pattern lists counts as the mic: a stream is
        // not tied to the browser whose window confirmed the call.
        (
            "Google Meet",
            "pw-firefox-mic.json",
            Some("clients-browsing.json"),
            Evidence::MicOnly,
        ),
        // A native client is its mic stream, whatever its windows show.
        (
            "Zoom",
            "pw-zoom.json",
            Some("clients-zoom.json"),
            Evidence::Full,
        ),
        ("Zoom", "pw-zoom.json", None, Evidence::Full),
        (
            "Zoom",
            "pw-idle.json",
            Some("clients-zoom.json"),
            Evidence::Gone,
        ),
        ("Zoom", "pw-idle.json", None, Evidence::Gone),
        // voxtype's own capture is nobody's mic.
        (
            "Google Meet",
            "pw-voxtype.json",
            Some("clients-browsing.json"),
            Evidence::Gone,
        ),
    ] {
        assert_eq!(
            evidence(app(name), &snap(pw, clients)),
            want,
            "{name} {pw} {clients:?}"
        );
    }
}

/// 2026-11-17, 21:01 to 21:06 UTC: a Zoom call in Chromium, recorded from
/// the prompt. Five minutes in, another tab came to the front of the
/// browser window, the launcher tab's title was gone, and the recording was
/// stopped as "Zoom (web) released the mic" with the call still under way
/// and Chromium still holding the mic.
#[test]
fn the_incident_another_tab_in_front_does_not_end_the_call() {
    let mut d = Detector::new(config());
    let joined = snap("pw-chromium-zoom-web.json", Some("clients-zoom-web.json"));
    let between_streams = snap("pw-idle.json", Some("clients-zoom-web.json"));
    let other_tab = snap(
        "pw-chromium-zoom-web.json",
        Some("clients-zoom-web-other-tab.json"),
    );
    let t0 = Instant::now();

    // 21:01:01 the pre-join stream, 21:01:03 gone, 21:01:05 the call's.
    let first = d.evaluate(&joined, IDLE, t0);
    assert!(
        matches!(first.as_slice(), [Event::Seen { call, .. }] if call.app == "Zoom (web)"),
        "{first:?}"
    );
    let gap = d.evaluate(&between_streams, IDLE, secs(t0, 2));
    assert!(matches!(gap.as_slice(), [Event::Brief { .. }]), "{gap:?}");
    let again = d.evaluate(&joined, IDLE, secs(t0, 4));
    assert!(
        matches!(again.as_slice(), [Event::Seen { .. }]),
        "{again:?}"
    );
    // 21:01:11 the prompt; 21:01:23 Record.
    let asked = watch(&mut d, &joined, IDLE, t0, 6, 22);
    assert_eq!(prompts(&asked).len(), 1, "{asked:?}");
    assert_eq!(prompts(&asked)[0].title, "Zoom call");
    assert!(watch(&mut d, &joined, RECORDING, t0, 22, 294).is_empty());

    // 21:05:55 another tab is in front. The call goes on for half an hour.
    let rest = watch(&mut d, &other_tab, RECORDING, t0, 294, 294 + 1800);
    assert_eq!(continues(&rest), vec![Evidence::MicOnly], "{rest:?}");
    assert!(acted(&rest).is_empty(), "{rest:?}");
    assert_eq!(
        describe(&rest[0]),
        "call continues: Zoom (web) still has the mic, but no window title shows the call \
         any more (another tab or page is in front)"
    );

    // Leaving the call: the mic is released and the web client's window
    // closes. That, after the debounce, is the end.
    let left = snap("pw-idle.json", Some("clients-browsing.json"));
    let t1 = 294 + 1800;
    assert!(watch(&mut d, &left, RECORDING, t0, t1, t1 + 4).is_empty());
    let end = d.evaluate(&left, RECORDING, secs(t0, t1 + 6));
    assert!(
        matches!(
            end.as_slice(),
            [Event::Ended {
                offer_stop: true,
                ..
            }]
        ),
        "{end:?}"
    );
    assert_eq!(
        describe(&end[0]),
        "call ended: Zoom (web) released the mic and no window shows the call any more \
         while a meeting records ([meeting.detect] on_call_end decides)"
    );
}

/// The launcher tab comes and goes as tabs are switched: each change is
/// said once, and none of them ends the call or offers it again.
#[test]
fn switching_tabs_back_and_forth_changes_nothing() {
    let mut d = Detector::new(config());
    let front = snap("pw-chromium-zoom-web.json", Some("clients-zoom-web.json"));
    let behind = snap(
        "pw-chromium-zoom-web.json",
        Some("clients-zoom-web-other-tab.json"),
    );
    let t0 = Instant::now();
    let mut all = watch(&mut d, &front, IDLE, t0, 0, 10);
    for round in 0..5 {
        let at = 10 + round * 120;
        all.extend(watch(&mut d, &behind, IDLE, t0, at, at + 60));
        all.extend(watch(&mut d, &front, IDLE, t0, at + 60, at + 120));
    }
    assert_eq!(prompts(&all).len(), 1, "{all:?}");
    assert_eq!(
        continues(&all),
        [Evidence::MicOnly, Evidence::Full].repeat(5),
        "{all:?}"
    );
    assert_eq!(acted(&all).len(), 2, "seen and prompted, once: {all:?}");
}

/// If a mute releases the browser's capture stream, the call window is
/// still there, and that is enough: no end, no second prompt.
#[test]
fn muting_with_the_call_window_open_does_not_end_the_call() {
    let t0 = Instant::now();
    let mut d = settled_meet_call(t0);
    let unmuted = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    let muted = snap("pw-idle.json", Some("clients-meet-chromium.json"));
    let mut all = Vec::new();
    for round in 0..4 {
        // Ten minutes muted, one unmuted.
        let at = 8 + round * 660;
        all.extend(watch(&mut d, &muted, RECORDING, t0, at, at + 600));
        all.extend(watch(&mut d, &unmuted, RECORDING, t0, at + 600, at + 660));
    }
    assert!(acted(&all).is_empty(), "{all:?}");
    assert_eq!(
        continues(&all),
        [Evidence::WindowOnly, Evidence::Full].repeat(4),
        "{all:?}"
    );
    assert_eq!(
        describe(&all[0]),
        "call continues: Google Meet released the mic, but its call window is still open \
         (muted, or reconnecting)"
    );
    assert_eq!(
        describe(&all[1]),
        "call continues: Google Meet has the mic and its call window again"
    );
}

/// The same with no meeting recording (the prompt was dismissed): the mic
/// coming back is not a new call to ask about.
#[test]
fn unmuting_does_not_prompt_a_second_time() {
    let t0 = Instant::now();
    let mut d = settled_meet_call(t0);
    let unmuted = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    let muted = snap("pw-idle.json", Some("clients-meet-chromium.json"));
    let mut all = watch(&mut d, &muted, IDLE, t0, 8, 308);
    all.extend(watch(&mut d, &unmuted, IDLE, t0, 308, 608));
    all.extend(watch(&mut d, &muted, IDLE, t0, 608, 908));
    all.extend(watch(&mut d, &unmuted, IDLE, t0, 908, 1208));
    assert!(acted(&all).is_empty(), "{all:?}");
}

/// A call recorded without asking is not started a second time either,
/// which would stop nothing but would announce the recording again.
#[test]
fn unmuting_does_not_auto_record_a_second_time() {
    let mut cfg = config();
    cfg.auto_record_apps = vec!["Google Meet".into()];
    let mut d = Detector::new(cfg);
    let unmuted = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    let muted = snap("pw-idle.json", Some("clients-meet-chromium.json"));
    let t0 = Instant::now();
    let mut all = watch(&mut d, &unmuted, IDLE, t0, 0, 10);
    for round in 0..3 {
        let at = 10 + round * 400;
        all.extend(watch(&mut d, &muted, RECORDING, t0, at, at + 300));
        all.extend(watch(&mut d, &unmuted, RECORDING, t0, at + 300, at + 400));
    }
    let started = all
        .iter()
        .filter(|e| matches!(e, Event::AutoRecord(_)))
        .count();
    assert_eq!(started, 1, "{all:?}");
    assert_eq!(acted(&all).len(), 2, "seen and recorded, once: {all:?}");
}

/// Muted, then the call's tab is closed: now nothing is left of the call,
/// and it ends after the debounce.
#[test]
fn closing_the_window_with_the_mic_released_ends_the_call() {
    let t0 = Instant::now();
    let mut d = settled_meet_call(t0);
    let muted = snap("pw-idle.json", Some("clients-meet-chromium.json"));
    let closed = snap("pw-idle.json", Some("clients-browsing.json"));
    let held = watch(&mut d, &muted, RECORDING, t0, 8, 608);
    assert!(acted(&held).is_empty(), "{held:?}");
    assert!(d.evaluate(&closed, RECORDING, secs(t0, 608)).is_empty());
    assert!(d.evaluate(&closed, RECORDING, secs(t0, 610)).is_empty());
    let end = d.evaluate(&closed, RECORDING, secs(t0, 612));
    assert!(
        matches!(
            end.as_slice(),
            [Event::Ended {
                offer_stop: true,
                ..
            }]
        ),
        "{end:?}"
    );
    // Said once.
    assert!(watch(&mut d, &closed, RECORDING, t0, 614, 700).is_empty());
    assert!(!d.is_tracking(), "forgotten once the grace period is over");
}

/// The call's window back within the grace period (20 s) is the same call:
/// the stop is called off ([`Event::Returned`]) and nothing is offered.
#[test]
fn the_window_back_within_the_grace_period_is_the_same_call() {
    let t0 = Instant::now();
    let mut d = settled_meet_call(t0);
    let unmuted = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    let muted = snap("pw-idle.json", Some("clients-meet-chromium.json"));
    let closed = snap("pw-idle.json", Some("clients-browsing.json"));
    watch(&mut d, &muted, RECORDING, t0, 8, 60);
    let end = watch(&mut d, &closed, RECORDING, t0, 60, 68);
    assert!(
        matches!(end.as_slice(), [Event::Ended { .. }]),
        "ended at 64: {end:?}"
    );
    // Twelve seconds into the grace period the tab is back, still muted.
    let back = d.evaluate(&muted, RECORDING, secs(t0, 76));
    assert!(
        matches!(back.as_slice(), [Event::Returned { call }] if call.app == "Google Meet"),
        "{back:?}"
    );
    assert_eq!(
        describe(&back[0]),
        "call is back: Google Meet is seen again within the grace period; the same call, \
         so nothing is offered again"
    );
    // And it is a call like before: it goes on muted, and unmuted.
    let mut rest = watch(&mut d, &muted, RECORDING, t0, 78, 600);
    rest.extend(watch(&mut d, &unmuted, IDLE, t0, 600, 700));
    assert!(acted(&rest).is_empty(), "{rest:?}");
    assert_eq!(continues(&rest), vec![Evidence::Full], "{rest:?}");
}

/// The mic alone coming back within the grace period is the same call too,
/// though a browser's mic with no call window would not start one.
#[test]
fn the_mic_back_within_the_grace_period_is_the_same_call() {
    let t0 = Instant::now();
    let mut d = settled_meet_call(t0);
    let closed = snap("pw-idle.json", Some("clients-browsing.json"));
    let mic_only = snap("pw-chromium-mic.json", Some("clients-browsing.json"));
    let end = watch(&mut d, &closed, IDLE, t0, 8, 16);
    assert!(
        matches!(
            end.as_slice(),
            [Event::Ended {
                offer_stop: false,
                ..
            }]
        ),
        "{end:?}"
    );
    let back = d.evaluate(&mic_only, IDLE, secs(t0, 24));
    assert!(
        matches!(back.as_slice(), [Event::Returned { .. }]),
        "{back:?}"
    );
    assert!(watch(&mut d, &mic_only, IDLE, t0, 26, 200).is_empty());
}

/// After the grace period the call is forgotten: the same app on the mic
/// with its window is a new call, and is offered.
#[test]
fn a_call_after_the_grace_period_is_a_new_call() {
    let t0 = Instant::now();
    let mut d = settled_meet_call(t0);
    let call = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    let closed = snap("pw-idle.json", Some("clients-browsing.json"));
    let mic_only = snap("pw-chromium-mic.json", Some("clients-browsing.json"));
    let end = watch(&mut d, &closed, IDLE, t0, 8, 40);
    assert!(matches!(end.as_slice(), [Event::Ended { .. }]), "{end:?}");
    assert!(!d.is_tracking());
    // The browser's mic with no call window starts nothing, as ever.
    assert!(watch(&mut d, &mic_only, IDLE, t0, 40, 60).is_empty());
    let next = watch(&mut d, &call, IDLE, t0, 60, 70);
    assert!(matches!(next[0], Event::Seen { .. }), "{next:?}");
    assert_eq!(prompts(&next).len(), 1, "{next:?}");
}

/// A native client has only its mic: it ends when it releases it, with its
/// windows still open, as before.
#[test]
fn a_native_call_ends_when_its_app_releases_the_mic() {
    let mut d = Detector::new(config());
    let call = snap("pw-zoom.json", Some("clients-zoom.json"));
    let released = snap("pw-idle.json", Some("clients-zoom.json"));
    let t0 = Instant::now();
    let start = watch(&mut d, &call, IDLE, t0, 0, 10);
    assert_eq!(prompts(&start).len(), 1, "{start:?}");
    assert!(watch(&mut d, &call, RECORDING, t0, 10, 300).is_empty());
    assert!(watch(&mut d, &released, RECORDING, t0, 300, 304).is_empty());
    let end = d.evaluate(&released, RECORDING, secs(t0, 306));
    assert!(
        matches!(
            end.as_slice(),
            [Event::Ended {
                offer_stop: true,
                ..
            }]
        ),
        "{end:?}"
    );
    assert_eq!(
        describe(&end[0]),
        "call ended: Zoom released the mic while a meeting records \
         ([meeting.detect] on_call_end decides)"
    );
}

/// Its mic back within the grace period calls the stop off and asks
/// nothing; back after it, it is a new call.
#[test]
fn a_native_app_back_on_the_mic_within_the_grace_period_is_the_same_call() {
    let mut d = Detector::new(config());
    let call = snap("pw-zoom.json", Some("clients-zoom.json"));
    let released = snap("pw-idle.json", Some("clients-zoom.json"));
    let t0 = Instant::now();
    watch(&mut d, &call, IDLE, t0, 0, 10);
    let end = watch(&mut d, &released, IDLE, t0, 10, 18);
    assert!(matches!(end.as_slice(), [Event::Ended { .. }]), "{end:?}");
    let back = watch(&mut d, &call, IDLE, t0, 30, 60);
    assert!(
        matches!(back.as_slice(), [Event::Returned { .. }]),
        "{back:?}"
    );

    let end = watch(&mut d, &released, IDLE, t0, 60, 68);
    assert!(matches!(end.as_slice(), [Event::Ended { .. }]), "{end:?}");
    assert!(watch(&mut d, &released, IDLE, t0, 68, 100).is_empty());
    let next = watch(&mut d, &call, IDLE, t0, 100, 110);
    assert!(matches!(next[0], Event::Seen { .. }), "{next:?}");
    assert_eq!(prompts(&next).len(), 1, "{next:?}");
}

/// With no grace period nothing is remembered, as before.
#[test]
fn without_a_grace_period_an_ended_call_is_forgotten_at_once() {
    let mut cfg = config();
    cfg.stop_grace_secs = 0;
    let mut d = Detector::new(cfg);
    let call = snap("pw-zoom.json", Some("clients-zoom.json"));
    let released = snap("pw-idle.json", Some("clients-zoom.json"));
    let t0 = Instant::now();
    watch(&mut d, &call, IDLE, t0, 0, 10);
    let end = watch(&mut d, &released, IDLE, t0, 10, 18);
    assert!(matches!(end.as_slice(), [Event::Ended { .. }]), "{end:?}");
    assert!(!d.is_tracking());
    let next = d.evaluate(&call, IDLE, secs(t0, 18));
    assert!(matches!(next.as_slice(), [Event::Seen { .. }]), "{next:?}");
}

/// Hyprland not answering is not the call window being closed: with the
/// mic released as well, the call still goes on until the windows can be
/// read again and show that it is gone.
#[test]
fn unreadable_windows_do_not_end_a_browser_call() {
    let t0 = Instant::now();
    let mut d = settled_meet_call(t0);
    let mic_no_windows = snap("pw-chromium-mic.json", None);
    let nothing_readable = snap("pw-idle.json", None);
    let closed = snap("pw-idle.json", Some("clients-browsing.json"));

    let mut all = watch(&mut d, &mic_no_windows, RECORDING, t0, 8, 68);
    all.extend(watch(&mut d, &nothing_readable, RECORDING, t0, 68, 1868));
    assert!(acted(&all).is_empty(), "{all:?}");
    assert_eq!(
        continues(&all),
        vec![
            Evidence::WindowsUnknown { mic: true },
            Evidence::WindowsUnknown { mic: false }
        ],
        "{all:?}"
    );
    assert_eq!(
        describe(&all[1]),
        "call continues: Google Meet released the mic, and window titles cannot be read \
         right now, so the call is not taken to be over"
    );

    // Readable again, and the window is gone: over, after the debounce.
    let end = watch(&mut d, &closed, RECORDING, t0, 1868, 1876);
    assert!(
        matches!(
            end.as_slice(),
            [Event::Ended {
                offer_stop: true,
                ..
            }]
        ),
        "{end:?}"
    );
}

/// Before it has settled a browser call still needs both: the mic without
/// the window is a browser using the mic, not a call.
#[test]
fn a_call_that_has_not_settled_needs_all_of_its_evidence() {
    let mut d = Detector::new(config());
    let call = snap("pw-chromium-mic.json", Some("clients-meet-chromium.json"));
    let mic_only = snap("pw-chromium-mic.json", Some("clients-browsing.json"));
    let t0 = Instant::now();
    d.evaluate(&call, IDLE, t0);
    let gone = d.evaluate(&mic_only, IDLE, secs(t0, 2));
    assert!(matches!(gone.as_slice(), [Event::Brief { .. }]), "{gone:?}");
    assert!(watch(&mut d, &mic_only, IDLE, t0, 4, 60).is_empty());
    assert!(!d.is_tracking());
}

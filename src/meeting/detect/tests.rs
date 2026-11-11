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
    assert_eq!(p[0].title, "Zoom Meeting");
}

#[test]
fn zoom_prompts_without_window_titles() {
    let mut d = Detector::new(config());
    let events = settle(&mut d, &snap("pw-zoom.json", None), IDLE);
    let p = prompts(&events);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].title, "Zoom");
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
    assert_eq!(p[0].title, "Meet - abc-defg-hij");
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
    assert_eq!(p[0].title, "Meet - abc-defg-hij");
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
    assert_eq!(p[0].title, "Weekly sync | Microsoft Teams");
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
        .any(|e| matches!(e, Event::AutoRecord(c) if c.title == "Zoom Meeting")));
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
        text.contains("would prompt: Record \"Meet - abc-defg-hij\"? (Google Meet)"),
        "{text}"
    );
}

#[test]
fn report_says_why_nothing_prompts() {
    let text = report(&snap("pw-idle.json", None), IDLE, &config()).join("\n");
    assert!(text.contains("no prompt: no call detected"), "{text}");
    assert!(text.contains("unavailable"), "{text}");
}

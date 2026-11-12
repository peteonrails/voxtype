//! `voxtype meeting actions`: a meeting's action items, and sending them
//! to Todoist or as Omarchy reminders.
//!
//! The items come from the meeting's summary: text, owner (a speaker name),
//! the deadline as said and resolved to a day, and where in the meeting it
//! was given. Every send is recorded per item, so sending again skips what
//! already went.

pub mod due;
pub mod reminder;
pub mod todoist;

use crate::meeting::data::{MeetingMetadata, MeetingSummary};
use crate::meeting::storage::{MeetingStorage, StorageError};
use crate::meeting::MeetingId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Where action items can go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Todoist,
    Reminder,
}

impl Target {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "todoist" => Some(Self::Todoist),
            "reminder" | "reminders" | "omarchy" => Some(Self::Reminder),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Todoist => "todoist",
            Self::Reminder => "reminder",
        }
    }
}

/// An item's identity across regenerated summaries: its text, ignoring
/// case and spacing.
pub fn item_key(text: &str) -> String {
    let normal = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let hash = Sha256::digest(normal.as_bytes());
    hash.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Whether `owner` is the user: "You", or `my_name` (whole or first name).
pub fn is_mine(owner: Option<&str>, my_name: Option<&str>) -> bool {
    let Some(owner) = owner.map(str::trim).filter(|o| !o.is_empty()) else {
        return false;
    };
    if ["you", "me", "myself"].contains(&owner.to_lowercase().as_str()) {
        return true;
    }
    let Some(name) = my_name.map(str::trim).filter(|n| !n.is_empty()) else {
        return false;
    };
    let first = |s: &str| s.split_whitespace().next().unwrap_or("").to_lowercase();
    owner.eq_ignore_ascii_case(name) || first(owner) == first(name)
}

/// Fill in each item's `due_on` from what was said, against the meeting's
/// local date. Items that already have one keep it.
pub fn resolve_dues(summary: &mut MeetingSummary, metadata: &MeetingMetadata) {
    let day = metadata
        .started_at
        .with_timezone(&chrono::Local)
        .date_naive();
    for item in &mut summary.action_items {
        if item.due_on.is_none() {
            item.due_on = item
                .due_date
                .as_deref()
                .and_then(|said| due::resolve(said, day))
                .map(|d| d.on);
        }
    }
}

/// One send of an item.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Sent {
    /// The Todoist task ID; none for a reminder
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Unix seconds
    pub at: i64,
}

/// An action item as `meeting actions` lists it.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    /// 1-based, what `--pick` takes
    pub number: usize,
    pub key: String,
    pub text: String,
    pub owner: Option<String>,
    pub mine: bool,
    /// The day it is due, YYYY-MM-DD
    pub due: Option<chrono::NaiveDate>,
    /// The time of day it is due, HH:MM, when one was said
    pub due_time: Option<String>,
    /// The deadline as it was said
    pub due_said: Option<String>,
    pub source_ms: Option<u64>,
    /// Where it was sent, by target
    pub sent: BTreeMap<String, Sent>,
    #[serde(skip)]
    due_at: Option<chrono::NaiveTime>,
}

/// A meeting's action items.
#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub meeting_id: String,
    pub title: String,
    /// Whether the meeting has a summary at all
    pub summarized: bool,
    pub items: Vec<Item>,
    #[serde(skip)]
    pub started_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip)]
    id: Option<MeetingId>,
}

/// The action items of meeting `id`, with what has been sent.
pub fn list(
    storage: &MeetingStorage,
    id: &MeetingId,
    my_name: Option<&str>,
) -> Result<Listing, StorageError> {
    let metadata = storage
        .get_meeting(id)?
        .ok_or_else(|| StorageError::NotFound(id.to_string()))?;
    let day = metadata
        .started_at
        .with_timezone(&chrono::Local)
        .date_naive();
    let mut sends: BTreeMap<String, BTreeMap<String, Sent>> = BTreeMap::new();
    for s in storage.action_sends(id)? {
        sends.entry(s.item_key).or_default().insert(
            s.target,
            Sent {
                id: s.remote_id,
                at: s.sent_at,
            },
        );
    }
    let summary = storage.get_summary(id)?;
    let items = summary
        .as_ref()
        .map(|s| s.action_items.as_slice())
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let key = item_key(&a.description);
            let resolved = a
                .due_date
                .as_deref()
                .and_then(|said| due::resolve(said, day));
            Item {
                number: i + 1,
                text: a.description.trim().to_string(),
                owner: a.assignee.clone(),
                mine: is_mine(a.assignee.as_deref(), my_name),
                due: a.due_on.or(resolved.map(|d| d.on)),
                due_time: resolved
                    .and_then(|d| d.at)
                    .map(|t| t.format("%H:%M").to_string()),
                due_at: resolved.and_then(|d| d.at),
                due_said: a.due_date.clone(),
                source_ms: a.source_ms,
                sent: sends.remove(&key).unwrap_or_default(),
                key,
            }
        })
        .collect();
    Ok(Listing {
        meeting_id: id.to_string(),
        title: metadata.display_title(),
        summarized: summary.is_some(),
        items,
        started_at: metadata.started_at,
        id: Some(*id),
    })
}

/// Item numbers from "1,3-4" (1-based, as listed), checked against `count`.
pub fn parse_pick(pick: &str, count: usize) -> Result<Vec<usize>, String> {
    let mut out = Vec::new();
    for part in pick.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (part, part),
        };
        let parse = |s: &str| {
            s.parse::<usize>()
                .map_err(|_| format!("'{part}' is not an item number"))
        };
        let (a, b) = (parse(a)?, parse(b)?);
        if a == 0 || b < a || b > count {
            return Err(format!("'{part}' is not among the items (1 to {count})"));
        }
        out.extend(a..=b);
    }
    if out.is_empty() {
        return Err("--pick names no items".into());
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// Which items a send takes.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// Item numbers; empty means all
    pub pick: Vec<usize>,
    /// Only the user's own
    pub mine: bool,
    /// Send again what was already sent there
    pub resend: bool,
}

impl Listing {
    /// The items `selection` takes, in order.
    pub fn select<'a>(&'a self, selection: &Selection) -> Vec<&'a Item> {
        self.items
            .iter()
            .filter(|i| selection.pick.is_empty() || selection.pick.contains(&i.number))
            .filter(|i| !selection.mine || i.mine)
            .collect()
    }
}

/// How sending one item went.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SendResult {
    pub number: usize,
    pub text: String,
    /// "sent", "already_sent" or "failed"
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// For a reminder, how many minutes away it is
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
}

/// What a send uses to reach each target.
pub struct Senders<'a> {
    pub todoist: Option<&'a dyn todoist::TodoistClient>,
    pub reminders: Option<&'a reminder::Reminders>,
    /// Todoist project for new tasks; Inbox when unset
    pub project_id: Option<String>,
    /// Reminder delay when no time today was said
    pub remind_in_mins: u32,
    pub now: chrono::NaiveDateTime,
}

/// The task an item becomes: its text (with the owner when it is someone
/// else's), its day or day and time, and where it came from.
pub fn task_for(listing: &Listing, item: &Item, project_id: Option<String>) -> todoist::NewTask {
    let mut content = item.text.clone();
    if let Some(owner) = item.owner.as_deref().filter(|_| !item.mine) {
        content.push_str(&format!(" ({owner})"));
    }
    let local = listing.started_at.with_timezone(&chrono::Local);
    let mut description = format!(
        "From the meeting \"{}\" on {}",
        listing.title,
        local.format("%B %-d, %Y")
    );
    if let Some(ms) = item.source_ms {
        description.push_str(&format!(
            " at {}",
            crate::meeting::assistant::prompt::ts(ms)
        ));
    }
    if let Some(said) = item.due_said.as_deref().filter(|_| item.due.is_none()) {
        description.push_str(&format!(".\nDue, as said: {said}"));
    }
    description.push_str(&format!(
        ".\nOpen it with: voxtype meeting view {}",
        &listing.meeting_id[..8.min(listing.meeting_id.len())]
    ));
    let (due_date, due_string) = match (item.due, &item.due_time) {
        (Some(d), Some(t)) => (None, Some(format!("{} {t}", d.format("%Y-%m-%d")))),
        (Some(d), None) => (Some(d.format("%Y-%m-%d").to_string()), None),
        _ => (None, None),
    };
    todoist::NewTask {
        content,
        description: Some(description),
        due_date,
        due_string,
        project_id,
    }
}

/// Send `items` to `target`, recording each that goes. One failure does
/// not stop the rest.
pub fn send(
    storage: &MeetingStorage,
    listing: &Listing,
    items: &[&Item],
    target: Target,
    senders: &Senders,
    resend: bool,
) -> Vec<SendResult> {
    let id = listing.id.expect("a listing from list()");
    let mut out = Vec::new();
    let mut last_minutes: Option<u32> = None;
    for item in items {
        let mut result = SendResult {
            number: item.number,
            text: item.text.clone(),
            status: "sent",
            remote_id: None,
            error: None,
            minutes: None,
        };
        if !resend {
            if let Some(sent) = item.sent.get(target.name()) {
                result.status = "already_sent";
                result.remote_id = sent.id.clone();
                out.push(result);
                continue;
            }
        }
        let sent: Result<Option<String>, String> = match target {
            Target::Todoist => match senders.todoist {
                Some(client) => client
                    .add_task(&task_for(listing, item, senders.project_id.clone()))
                    .map(Some)
                    .map_err(|e| e.to_string()),
                None => Err(todoist::TodoistError::NotConnected.to_string()),
            },
            Target::Reminder => match senders.reminders {
                Some(r) => {
                    let minutes = reminder::minutes_for(
                        item.due,
                        item.due_at,
                        senders.now,
                        senders.remind_in_mins,
                    );
                    if last_minutes == Some(minutes) {
                        std::thread::sleep(r.spacing);
                    }
                    last_minutes = Some(minutes);
                    result.minutes = Some(minutes);
                    r.set(minutes, &format!("{} (from {})", item.text, listing.title))
                        .map(|_| None)
                }
                None => Err("omarchy-reminder is not installed".into()),
            },
        };
        match sent {
            Ok(remote_id) => {
                if let Err(e) =
                    storage.record_action_sent(&id, &item.key, target.name(), remote_id.as_deref())
                {
                    tracing::warn!("Sent, but could not record it: {e}");
                }
                result.remote_id = remote_id;
            }
            Err(e) => {
                result.status = "failed";
                result.error = Some(e);
            }
        }
        out.push(result);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting::data::ActionItem;

    #[test]
    fn keys_ignore_case_and_spacing() {
        assert_eq!(item_key("Tag the release"), item_key("  tag  THE release "));
        assert_ne!(
            item_key("Tag the release"),
            item_key("Tag the release notes")
        );
        assert_eq!(item_key("x").len(), 16);
    }

    #[test]
    fn mine_is_you_or_my_name() {
        assert!(is_mine(Some("You"), None));
        assert!(is_mine(Some("Pete"), Some("Pete Jackson")));
        assert!(is_mine(Some("pete jackson"), Some("Pete")));
        assert!(!is_mine(Some("Dana"), Some("Pete")));
        assert!(!is_mine(Some("Pete"), None));
        assert!(!is_mine(None, Some("Pete")));
    }

    #[test]
    fn picks_are_numbers_and_ranges() {
        assert_eq!(parse_pick("1,3-4", 5).unwrap(), [1, 3, 4]);
        assert_eq!(parse_pick("2, 2,1", 2).unwrap(), [1, 2]);
        assert!(parse_pick("0", 3).is_err());
        assert!(parse_pick("4", 3).unwrap_err().contains("1 to 3"));
        assert!(parse_pick("a", 3).is_err());
        assert!(parse_pick("", 3).is_err());
        assert!(parse_pick("3-1", 3).is_err());
    }

    fn storage() -> (MeetingStorage, tempfile::TempDir, MeetingId) {
        let dir = tempfile::tempdir().unwrap();
        let storage = MeetingStorage::open(crate::meeting::StorageConfig {
            storage_path: dir.path().to_path_buf(),
            retain_audio: false,
            max_meetings: 0,
            my_name: None,
        })
        .unwrap();
        let mut metadata = MeetingMetadata::new(Some("Release sync".into()));
        // A Thursday afternoon, local time.
        metadata.started_at = chrono::NaiveDate::from_ymd_opt(2026, 11, 12)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap()
            .and_local_timezone(chrono::Local)
            .unwrap()
            .with_timezone(&chrono::Utc);
        storage.create_meeting(&metadata).unwrap();
        let item =
            |text: &str, owner: Option<&str>, due: Option<&str>, at: Option<u64>| ActionItem {
                description: text.into(),
                assignee: owner.map(Into::into),
                due_date: due.map(Into::into),
                source_ms: at,
                ..Default::default()
            };
        let mut summary = MeetingSummary {
            summary: "Shipping.".into(),
            key_points: vec![],
            action_items: vec![
                item(
                    "Tag the release",
                    Some("Dana"),
                    Some("Friday"),
                    Some(65_000),
                ),
                item("Write the notes", Some("You"), Some("by 3pm"), None),
                item("Book the room", None, Some("next week"), None),
            ],
            decisions: vec![],
            chapters: vec![],
            generated_at: chrono::Utc::now(),
            model: None,
        };
        resolve_dues(&mut summary, &metadata);
        storage.save_summary(&metadata.id, &summary).unwrap();
        (storage, dir, metadata.id)
    }

    #[test]
    fn items_list_with_owner_due_and_place() {
        let (storage, _dir, id) = storage();
        let listing = list(&storage, &id, Some("Pete")).unwrap();
        assert!(listing.summarized);
        let [tag, notes, room] = &listing.items[..] else {
            panic!("{listing:?}")
        };
        assert_eq!((tag.number, tag.text.as_str()), (1, "Tag the release"));
        assert_eq!(tag.due.unwrap().to_string(), "2026-11-13");
        assert_eq!(tag.source_ms, Some(65_000));
        assert!(!tag.mine);
        assert!(notes.mine);
        assert_eq!(notes.due.unwrap().to_string(), "2026-11-12");
        assert_eq!(notes.due_time.as_deref(), Some("15:00"));
        assert_eq!(room.due, None);
        assert_eq!(room.due_said.as_deref(), Some("next week"));
        // Stored with the summary, so `meeting show --json` has it too.
        let stored = storage.get_summary(&id).unwrap().unwrap();
        assert_eq!(stored.action_items[0].due_on, tag.due);

        let json = serde_json::to_value(&listing).unwrap();
        assert_eq!(json["items"][0]["due"], "2026-11-13");
        assert_eq!(json["items"][0]["owner"], "Dana");
        assert_eq!(json["items"][1]["due_time"], "15:00");
        assert_eq!(json["items"][2]["due"], serde_json::Value::Null);

        let selected = listing.select(&Selection {
            mine: true,
            ..Default::default()
        });
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].number, 2);
        let picked = listing.select(&Selection {
            pick: vec![1, 3],
            ..Default::default()
        });
        assert_eq!(picked.iter().map(|i| i.number).collect::<Vec<_>>(), [1, 3]);
    }

    fn senders<'a>(
        todoist: Option<&'a dyn todoist::TodoistClient>,
        reminders: Option<&'a reminder::Reminders>,
    ) -> Senders<'a> {
        Senders {
            todoist,
            reminders,
            project_id: None,
            remind_in_mins: 60,
            now: chrono::NaiveDate::from_ymd_opt(2026, 11, 12)
                .unwrap()
                .and_hms_opt(14, 30, 0)
                .unwrap(),
        }
    }

    #[test]
    fn all_items_go_to_todoist_once() {
        let (storage, _dir, id) = storage();
        let listing = list(&storage, &id, Some("Pete")).unwrap();
        let fake = todoist::FakeTodoist::default();
        let all = listing.select(&Selection::default());
        let results = send(
            &storage,
            &listing,
            &all,
            Target::Todoist,
            &senders(Some(&fake), None),
            false,
        );
        assert!(results.iter().all(|r| r.status == "sent"), "{results:?}");
        let tasks = fake.added.borrow().clone();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].content, "Tag the release (Dana)");
        assert_eq!(tasks[0].due_date.as_deref(), Some("2026-11-13"));
        let d = tasks[0].description.as_deref().unwrap();
        assert!(
            d.starts_with("From the meeting \"Release sync\" on November 12, 2026 at 01:05."),
            "{d}"
        );
        assert!(d.contains(&format!("voxtype meeting view {}", &id.to_string()[..8])));
        assert_eq!(tasks[1].content, "Write the notes", "your own: no owner");
        assert_eq!(tasks[1].due_string.as_deref(), Some("2026-11-12 15:00"));
        assert_eq!(tasks[1].due_date, None);
        assert_eq!(tasks[2].due_date, None, "next week names no day");
        assert!(tasks[2]
            .description
            .as_deref()
            .unwrap()
            .contains("Due, as said: next week"));

        // Sending again sends nothing new.
        let listing = list(&storage, &id, Some("Pete")).unwrap();
        assert_eq!(
            listing.items[0].sent["todoist"].id.as_deref(),
            Some("task-1")
        );
        let all = listing.select(&Selection::default());
        let again = send(
            &storage,
            &listing,
            &all,
            Target::Todoist,
            &senders(Some(&fake), None),
            false,
        );
        assert!(again.iter().all(|r| r.status == "already_sent"));
        assert_eq!(fake.added.borrow().len(), 3);
        // Unless asked.
        let forced = send(
            &storage,
            &listing,
            &all[..1],
            Target::Todoist,
            &senders(Some(&fake), None),
            true,
        );
        assert_eq!(forced[0].status, "sent");
        assert_eq!(fake.added.borrow().len(), 4);
    }

    #[test]
    fn a_failure_is_reported_and_not_recorded() {
        let (storage, _dir, id) = storage();
        let listing = list(&storage, &id, None).unwrap();
        let fake = todoist::FakeTodoist {
            fail_on: Some("notes".into()),
            ..Default::default()
        };
        let all = listing.select(&Selection::default());
        let results = send(
            &storage,
            &listing,
            &all,
            Target::Todoist,
            &senders(Some(&fake), None),
            false,
        );
        let statuses: Vec<&str> = results.iter().map(|r| r.status).collect();
        assert_eq!(statuses, ["sent", "failed", "sent"]);
        assert!(results[1].error.as_deref().unwrap().contains("bad task"));
        let listing = list(&storage, &id, None).unwrap();
        assert!(
            listing.items[1].sent.is_empty(),
            "a failed item can be sent again"
        );

        // Not connected: every item fails with the fix.
        let none = send(
            &storage,
            &listing,
            &all,
            Target::Todoist,
            &senders(None, None),
            false,
        );
        assert!(none[1]
            .error
            .as_deref()
            .unwrap()
            .contains("todoist connect"));
    }

    #[test]
    fn reminders_go_off_at_a_time_said_for_today_or_after_the_delay() {
        let (storage, dir, id) = storage();
        let listing = list(&storage, &id, Some("Pete")).unwrap();
        let program = dir.path().join("omarchy-reminder");
        crate::test_support::write_script(
            &program,
            &format!(
                "#!/bin/sh\necho \"$1|$2\" >> {}/calls\n",
                dir.path().display()
            ),
        );
        let reminders = reminder::Reminders {
            program,
            spacing: std::time::Duration::ZERO,
        };
        let all = listing.select(&Selection::default());
        let results = send(
            &storage,
            &listing,
            &all,
            Target::Reminder,
            &senders(None, Some(&reminders)),
            false,
        );
        assert_eq!(
            results.iter().map(|r| r.minutes).collect::<Vec<_>>(),
            [Some(60), Some(30), Some(60)]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("calls")).unwrap(),
            "60|Tag the release (from Release sync)\n\
             30|Write the notes (from Release sync)\n\
             60|Book the room (from Release sync)\n"
        );
        let listing = list(&storage, &id, Some("Pete")).unwrap();
        assert!(listing
            .items
            .iter()
            .all(|i| i.sent.contains_key("reminder")));
        assert!(listing
            .items
            .iter()
            .all(|i| !i.sent.contains_key("todoist")));
    }
}

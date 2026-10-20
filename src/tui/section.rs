//! The pages listed down the left side.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Basics,
    Engine,
    Hotkey,
    Audio,
    Output,
    Text,
    Vad,
    Meeting,
    Notifications,
    Osd,
    Waybar,
    Advanced,
    Build,
}

impl Section {
    /// Page order: the essentials, then the path a dictation takes, then the
    /// status around it, then everything else.
    pub const ALL: &'static [Section] = &[
        Section::Basics,
        Section::Engine,
        Section::Hotkey,
        Section::Audio,
        Section::Vad,
        Section::Output,
        Section::Text,
        Section::Notifications,
        Section::Osd,
        Section::Waybar,
        Section::Meeting,
        Section::Advanced,
        Section::Build,
    ];

    /// The page's name, in the list and on its pane.
    pub const fn label(self) -> &'static str {
        match self {
            Section::Basics => "Basics",
            Section::Engine => "Engine",
            Section::Hotkey => "Hotkey",
            Section::Audio => "Audio",
            Section::Output => "Output",
            Section::Text => "Text",
            Section::Vad => "Voice detection",
            Section::Meeting => "Meeting",
            Section::Notifications => "Notifications",
            Section::Osd => "On-screen display",
            Section::Waybar => "Waybar",
            Section::Advanced => "Advanced",
            Section::Build => "Build",
        }
    }

    /// Pages in the same group sit together in the list; a blank line
    /// separates groups.
    pub const fn group(self) -> u8 {
        match self {
            Section::Basics => 0,
            Section::Engine
            | Section::Hotkey
            | Section::Audio
            | Section::Vad
            | Section::Output
            | Section::Text => 1,
            Section::Notifications | Section::Osd | Section::Waybar => 2,
            Section::Meeting | Section::Advanced | Section::Build => 3,
        }
    }

    /// What the page covers, shown under `?`.
    pub const fn summary(self) -> &'static str {
        match self {
            Section::Basics => "The settings most people change",
            Section::Engine => "Model, language and tuning for the engine",
            Section::Hotkey => "Listener, hotkey, mode, cancel key",
            Section::Audio => "Microphone, recording limit, media, sounds",
            Section::Vad => "Skipping silence before transcription",
            Section::Output => "How text reaches the focused window",
            Section::Text => "Spoken punctuation and replacements",
            Section::Notifications => "Desktop notifications",
            Section::Osd => "The overlay shown while recording",
            Section::Waybar => "Status bar icons",
            Section::Meeting => "Meeting transcription",
            Section::Advanced => "Eager processing, GPU device, streaming",
            Section::Build => "Which voxtype build is installed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list puts a blank line wherever the group changes, so a group
    /// split across two runs would be drawn as two groups.
    #[test]
    fn each_group_is_one_contiguous_run() {
        let mut seen: Vec<u8> = Vec::new();
        for section in Section::ALL {
            let group = section.group();
            if seen.last() != Some(&group) {
                assert!(!seen.contains(&group), "group {group} appears twice");
                seen.push(group);
            }
        }
    }

    #[test]
    fn every_page_is_listed_once() {
        for (i, a) in Section::ALL.iter().enumerate() {
            for b in &Section::ALL[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert_eq!(Section::ALL.len(), 13);
    }
}

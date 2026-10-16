//! The set of configuration sections shown in the sidebar.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    General,
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
    Streaming,
}

impl Section {
    pub const ALL: &'static [Section] = &[
        Section::General,
        Section::Engine,
        Section::Hotkey,
        Section::Audio,
        Section::Output,
        Section::Text,
        Section::Vad,
        Section::Meeting,
        Section::Notifications,
        Section::Osd,
        Section::Waybar,
        Section::Advanced,
        Section::Streaming,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Engine => "Engine",
            Section::Hotkey => "Hotkey",
            Section::Audio => "Audio",
            Section::Output => "Output",
            Section::Text => "Text",
            Section::Vad => "VAD",
            Section::Meeting => "Meeting",
            Section::Notifications => "Notifications",
            Section::Osd => "OSD",
            Section::Waybar => "Waybar",
            Section::Advanced => "Advanced",
            Section::Streaming => "Streaming",
        }
    }

    /// One-line description shown when the cursor is on the section in the
    /// sidebar but the section hasn't been opened yet.
    pub const fn summary(self) -> &'static str {
        match self {
            Section::General => "Engine, variant binary, daemon status",
            Section::Engine => "Engine + model + per-engine tuning",
            Section::Hotkey => "Push-to-talk key, mode, modifier, cancel key",
            Section::Audio => "Input device, max duration, feedback, MPRIS",
            Section::Output => "Mode, driver order, post-processing, profiles",
            Section::Text => "Spoken punctuation, replacements",
            Section::Vad => "Silero VAD, energy thresholds, eager processing",
            Section::Meeting => "Meeting mode: audio source, diarization, summary",
            Section::Notifications => "Desktop notifications and expire times",
            Section::Osd => "Floating waveform OSD: frontend, position, size, opacity",
            Section::Waybar => "Status integration: icon theme, overrides",
            Section::Advanced => "GPU isolation, flash attention, on-demand loading",
            Section::Streaming => "Live partial transcription: on/off + tick interval, buffer, revision mode",
        }
    }
}

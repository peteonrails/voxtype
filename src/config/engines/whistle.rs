//! Whistle engine configuration.

use serde::{Deserialize, Serialize};

use super::super::default_on_demand_loading;

/// Whistle speech-to-text configuration (Cactus Compute Whistle, run by the
/// `needle` CLI runner as an external process).
///
/// Whistle is a 16.9 MB on-device ASR model in seven languages
/// (en, de, fr, es, it, nl, pl). It runs on Cactus Compute's `needle` engine;
/// voxtype drives that engine's CLI runner and reads the transcript from its
/// JSON output, the same shape as the whisper-cli backend.
///
/// Both artifacts are fetched separately — the engine is not bundled:
///   - the runner: https://huggingface.co/Cactus-Compute/needle3
///     (`linux-x86_64/needle`, `macos-arm64/needle`, ...)
///   - the weights: https://huggingface.co/Cactus-Compute/whistle
///     (`whistle.cact`)
///
/// Whistle telemetry is on by default in the runner; voxtype always launches
/// it with `NEEDLE_TELEMETRY=0` and `DO_NOT_TRACK=1` so nothing leaves the box.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WhistleConfig {
    /// Model name or path to the `whistle.cact` weights. A short name is
    /// resolved in voxtype's models directory; an absolute path is used as is.
    ///
    /// Defaults in serde rather than being seeded from `model_catalog` (the
    /// way the ONNX engines' `model` is), because whistle.cact isn't served
    /// from models.voxtype.io — a `[whistle]` table that only sets, say,
    /// `language` still has to load.
    #[serde(default = "default_whistle_model")]
    pub model: String,

    /// Path to the `needle` runner binary. When unset, `needle` is looked up
    /// on `PATH` and in the usual local-build locations.
    #[serde(default)]
    pub runner_path: Option<std::path::PathBuf>,

    /// Force the language ("en", "de", "fr", "es", "it", "nl", "pl").
    /// Empty (the default) lets Whistle detect it.
    #[serde(default)]
    pub language: String,

    /// File of words and phrases to favour, one per line (keyword biasing).
    #[serde(default)]
    pub keywords_file: Option<std::path::PathBuf>,

    /// Ask the runner to include per-word timestamps in its JSON output.
    #[serde(default)]
    pub word_timestamps: bool,

    /// Worker threads for the runner. `None`/`Some(0)` lets it pick.
    #[serde(default)]
    pub threads: Option<usize>,

    /// Load model on-demand when recording starts (true) or keep loaded (false)
    #[serde(default = "default_on_demand_loading")]
    pub on_demand_loading: bool,
}

fn default_whistle_model() -> String {
    "whistle.cact".to_string()
}

impl Default for WhistleConfig {
    fn default() -> Self {
        Self {
            model: default_whistle_model(),
            runner_path: None,
            language: String::new(),
            keywords_file: None,
            word_timestamps: false,
            threads: None,
            on_demand_loading: false,
        }
    }
}

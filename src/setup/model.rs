//! Interactive model selection and download

use super::{print_failure, print_info, print_success, print_warning};
use crate::config::{Config, TranscriptionEngine};
use crate::transcribe::whisper::{get_model_filename, get_model_url};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

/// Model information for display
struct ModelInfo {
    name: &'static str,
    size_mb: u32,
    description: &'static str,
    english_only: bool,
}

const MODELS: &[ModelInfo] = &[
    // Tiny models
    ModelInfo {
        name: "tiny",
        size_mb: 75,
        description: "Fastest, lowest accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "tiny.en",
        size_mb: 39,
        description: "Fastest, lowest accuracy",
        english_only: true,
    },
    // Base models
    ModelInfo {
        name: "base",
        size_mb: 142,
        description: "Good balance (default)",
        english_only: false,
    },
    ModelInfo {
        name: "base.en",
        size_mb: 142,
        description: "Good balance (default)",
        english_only: true,
    },
    // Small models
    ModelInfo {
        name: "small",
        size_mb: 466,
        description: "Better accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "small.en",
        size_mb: 466,
        description: "Better accuracy",
        english_only: true,
    },
    // Medium models
    ModelInfo {
        name: "medium",
        size_mb: 1500,
        description: "High accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "medium.en",
        size_mb: 1500,
        description: "High accuracy",
        english_only: true,
    },
    // Large models
    ModelInfo {
        name: "large-v3",
        size_mb: 3100,
        description: "Best accuracy",
        english_only: false,
    },
    ModelInfo {
        name: "large-v3-turbo",
        size_mb: 1600,
        description: "Fast + accurate (recommended for GPU)",
        english_only: false,
    },
];

// =============================================================================
// Parakeet Model Definitions
// =============================================================================

/// Parakeet model information for display and download
struct ParakeetModelInfo {
    name: &'static str,
    size_mb: u32,
    description: &'static str,
    files: &'static [(&'static str, u64)], // (filename, expected_size_bytes)
    huggingface_repo: &'static str,
}

const PARAKEET_MODELS: &[ParakeetModelInfo] = &[
    ParakeetModelInfo {
        name: "parakeet-tdt-0.6b-v3",
        size_mb: 2600,
        description: "TDT model with punctuation (recommended)",
        files: &[
            ("encoder-model.onnx", 43_825_971),
            ("encoder-model.onnx.data", 2_620_260_352),
            ("decoder_joint-model.onnx", 76_023_939),
            ("vocab.txt", 96_179),
            ("config.json", 97),
        ],
        huggingface_repo: "istupakov/parakeet-tdt-0.6b-v3-onnx",
    },
    ParakeetModelInfo {
        name: "parakeet-tdt-0.6b-v3-int8",
        size_mb: 670,
        description: "TDT quantized, smaller/faster",
        files: &[
            ("encoder-model.int8.onnx", 683_671_552),
            ("decoder_joint-model.int8.onnx", 19_087_667),
            ("vocab.txt", 96_179),
            ("config.json", 97),
        ],
        huggingface_repo: "istupakov/parakeet-tdt-0.6b-v3-onnx",
    },
];

// =============================================================================
// Moonshine Model Definitions
// =============================================================================

/// Moonshine model information for display and download
struct MoonshineModelInfo {
    /// Short config name (e.g., "base", "tiny", "base-ja")
    name: &'static str,
    /// Directory name under models/ (e.g., "moonshine-base")
    dir_name: &'static str,
    size_mb: u32,
    description: &'static str,
    /// Language display string (e.g., "en", "ja")
    language: &'static str,
    /// "MIT" for English models, "Community" for non-English (non-commercial only)
    license: &'static str,
    /// (repo_path, local_filename) - repo_path is the path within the HuggingFace repo
    files: &'static [(&'static str, &'static str)],
    huggingface_repo: &'static str,
}

const MOONSHINE_MODELS: &[MoonshineModelInfo] = &[
    // English models (MIT license)
    MoonshineModelInfo {
        name: "base",
        dir_name: "moonshine-base",
        size_mb: 237,
        description: "Fast, good accuracy (recommended)",
        language: "en",
        license: "MIT",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-base-ONNX",
    },
    MoonshineModelInfo {
        name: "tiny",
        dir_name: "moonshine-tiny",
        size_mb: 100,
        description: "Fastest, lower accuracy",
        language: "en",
        license: "MIT",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-tiny-ONNX",
    },
    // Multilingual models (Moonshine Community License - non-commercial only)
    MoonshineModelInfo {
        name: "base-ja",
        dir_name: "moonshine-base-ja",
        size_mb: 237,
        description: "Japanese",
        language: "ja",
        license: "Community",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-base-ja-ONNX",
    },
    MoonshineModelInfo {
        name: "base-zh",
        dir_name: "moonshine-base-zh",
        size_mb: 237,
        description: "Mandarin Chinese",
        language: "zh",
        license: "Community",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-base-zh-ONNX",
    },
    MoonshineModelInfo {
        name: "tiny-ja",
        dir_name: "moonshine-tiny-ja",
        size_mb: 100,
        description: "Japanese (tiny)",
        language: "ja",
        license: "Community",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-tiny-ja-ONNX",
    },
    MoonshineModelInfo {
        name: "tiny-zh",
        dir_name: "moonshine-tiny-zh",
        size_mb: 100,
        description: "Mandarin Chinese (tiny)",
        language: "zh",
        license: "Community",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-tiny-zh-ONNX",
    },
    MoonshineModelInfo {
        name: "tiny-ko",
        dir_name: "moonshine-tiny-ko",
        size_mb: 100,
        description: "Korean (tiny)",
        language: "ko",
        license: "Community",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-tiny-ko-ONNX",
    },
    MoonshineModelInfo {
        name: "tiny-ar",
        dir_name: "moonshine-tiny-ar",
        size_mb: 100,
        description: "Arabic (tiny)",
        language: "ar",
        license: "Community",
        files: &[
            ("onnx/encoder_model.onnx", "encoder_model.onnx"),
            (
                "onnx/decoder_model_merged.onnx",
                "decoder_model_merged.onnx",
            ),
            ("tokenizer.json", "tokenizer.json"),
        ],
        huggingface_repo: "onnx-community/moonshine-tiny-ar-ONNX",
    },
];

// =============================================================================
// SenseVoice Model Definitions
// =============================================================================

/// SenseVoice model information for display and download
struct SenseVoiceModelInfo {
    name: &'static str,
    dir_name: &'static str,
    size_mb: u32,
    description: &'static str,
    languages: &'static str,
    files: &'static [(&'static str, &'static str)], // (repo_path, local_filename)
    huggingface_repo: &'static str,
}

const SENSEVOICE_MODELS: &[SenseVoiceModelInfo] = &[
    SenseVoiceModelInfo {
        name: "small",
        dir_name: "sensevoice-small",
        size_mb: 239,
        description: "Quantized int8 (recommended)",
        languages: "zh/en/ja/ko/yue",
        files: &[
            ("model.int8.onnx", "model.int8.onnx"),
            ("tokens.txt", "tokens.txt"),
        ],
        huggingface_repo: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
    },
    SenseVoiceModelInfo {
        name: "small-fp32",
        dir_name: "sensevoice-small-fp32",
        size_mb: 938,
        description: "Full precision (larger, slightly better accuracy)",
        languages: "zh/en/ja/ko/yue",
        files: &[
            ("model.onnx", "model.onnx"),
            ("tokens.txt", "tokens.txt"),
        ],
        huggingface_repo: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
    },
];

// =============================================================================
// Paraformer Model Definitions
// =============================================================================

/// Paraformer model info (same structure as SenseVoice: model.onnx + tokens.txt)
struct ParaformerModelInfo {
    name: &'static str,
    dir_name: &'static str,
    size_mb: u32,
    description: &'static str,
    languages: &'static str,
    files: &'static [(&'static str, &'static str)],
    huggingface_repo: &'static str,
}

const PARAFORMER_MODELS: &[ParaformerModelInfo] = &[
    ParaformerModelInfo {
        name: "zh",
        dir_name: "paraformer-zh",
        size_mb: 487,
        description: "Chinese + English offline (recommended)",
        languages: "zh/en",
        files: &[
            ("model.int8.onnx", "model.int8.onnx"),
            ("tokens.txt", "tokens.txt"),
        ],
        huggingface_repo: "csukuangfj/sherpa-onnx-paraformer-zh-2023-09-14",
    },
    ParaformerModelInfo {
        name: "en",
        dir_name: "paraformer-en",
        size_mb: 220,
        description: "English offline",
        languages: "en",
        files: &[
            ("model.int8.onnx", "model.int8.onnx"),
            ("tokens.txt", "tokens.txt"),
        ],
        huggingface_repo: "csukuangfj/sherpa-onnx-paraformer-en-2024-03-09",
    },
];

// =============================================================================
// Dolphin Model Definitions
// =============================================================================

struct DolphinModelInfo {
    name: &'static str,
    dir_name: &'static str,
    size_mb: u32,
    description: &'static str,
    languages: &'static str,
    files: &'static [(&'static str, &'static str)],
    huggingface_repo: &'static str,
}

const DOLPHIN_MODELS: &[DolphinModelInfo] = &[
    DolphinModelInfo {
        name: "base",
        dir_name: "dolphin-base",
        size_mb: 198,
        description: "Dictation-optimized (recommended)",
        languages: "en/zh",
        files: &[
            ("model.int8.onnx", "model.int8.onnx"),
            ("tokens.txt", "tokens.txt"),
        ],
        huggingface_repo: "csukuangfj/sherpa-onnx-dolphin-base-ctc-multi-lang-int8-2025-04-02",
    },
];

// =============================================================================
// Omnilingual Model Definitions
// =============================================================================

struct OmnilingualModelInfo {
    name: &'static str,
    dir_name: &'static str,
    size_mb: u32,
    description: &'static str,
    languages: &'static str,
    files: &'static [(&'static str, &'static str)],
    huggingface_repo: &'static str,
}

const OMNILINGUAL_MODELS: &[OmnilingualModelInfo] = &[
    OmnilingualModelInfo {
        name: "300m",
        dir_name: "omnilingual-300m",
        size_mb: 3900,
        description: "1600+ languages, 300M params",
        languages: "1600+ langs",
        files: &[
            ("model.onnx", "model.onnx"),
            ("tokens.txt", "tokens.txt"),
        ],
        huggingface_repo: "csukuangfj/sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-2025-11-12",
    },
];

// =============================================================================
// Whisper Model Functions
// =============================================================================

/// Check if a model name is valid (Whisper models)
pub fn is_valid_model(name: &str) -> bool {
    MODELS.iter().any(|m| m.name == name)
}

/// Get list of valid model names (for error messages)
pub fn valid_model_names() -> Vec<&'static str> {
    MODELS.iter().map(|m| m.name).collect()
}

/// Run interactive model selection (single menu with all models)
pub async fn interactive_select() -> anyhow::Result<()> {
    println!("Voxtype Model Selection\n");
    println!("=======================\n");

    let models_dir = Config::models_dir();
    println!("Models directory: {:?}\n", models_dir);

    // Load current config to determine active model
    let config = crate::config::load_config(Config::default_path().as_deref()).unwrap_or_default();
    let is_whisper_engine = matches!(config.engine, TranscriptionEngine::Whisper);
    let is_parakeet_engine = matches!(config.engine, TranscriptionEngine::Parakeet);
    let is_moonshine_engine = matches!(config.engine, TranscriptionEngine::Moonshine);
    let is_sensevoice_engine = matches!(config.engine, TranscriptionEngine::SenseVoice);
    let is_paraformer_engine = matches!(config.engine, TranscriptionEngine::Paraformer);
    let is_dolphin_engine = matches!(config.engine, TranscriptionEngine::Dolphin);
    let is_omnilingual_engine = matches!(config.engine, TranscriptionEngine::Omnilingual);
    let current_whisper_model = &config.whisper.model;
    let current_parakeet_model = config.parakeet.as_ref().map(|p| p.model.as_str());
    let current_moonshine_model = config.moonshine.as_ref().map(|m| m.model.as_str());
    let current_sensevoice_model = config.sensevoice.as_ref().map(|s| s.model.as_str());
    let current_paraformer_model = config.paraformer.as_ref().map(|p| p.model.as_str());
    let current_dolphin_model = config.dolphin.as_ref().map(|d| d.model.as_str());
    let current_omnilingual_model = config.omnilingual.as_ref().map(|o| o.model.as_str());
    let parakeet_available = cfg!(feature = "parakeet");
    let moonshine_available = cfg!(feature = "moonshine");
    let sensevoice_available = cfg!(feature = "sensevoice");
    let paraformer_available = cfg!(feature = "paraformer");
    let dolphin_available = cfg!(feature = "dolphin");
    let omnilingual_available = cfg!(feature = "omnilingual");
    let whisper_count = MODELS.len();
    let parakeet_count = PARAKEET_MODELS.len();
    let moonshine_count = MOONSHINE_MODELS.len();
    let sensevoice_count = SENSEVOICE_MODELS.len();
    let paraformer_count = PARAFORMER_MODELS.len();
    let dolphin_count = DOLPHIN_MODELS.len();
    let omnilingual_count = OMNILINGUAL_MODELS.len();

    let available_count = |available: bool, count: usize| if available { count } else { 0 };
    let total_count = whisper_count
        + available_count(parakeet_available, parakeet_count)
        + available_count(moonshine_available, moonshine_count)
        + available_count(sensevoice_available, sensevoice_count)
        + available_count(paraformer_available, paraformer_count)
        + available_count(dolphin_available, dolphin_count)
        + available_count(omnilingual_available, omnilingual_count);

    // --- Whisper Section ---
    println!("--- Whisper (OpenAI, 99+ languages) ---\n");

    for (i, model) in MODELS.iter().enumerate() {
        let filename = get_model_filename(model.name);
        let model_path = models_dir.join(&filename);
        let installed = model_path.exists();

        let is_current = is_whisper_engine && model.name == current_whisper_model;
        let star = if is_current { "*" } else { " " };

        let status = if installed {
            "\x1b[32m[installed]\x1b[0m"
        } else {
            ""
        };

        let lang = if model.english_only { "en" } else { "multi" };

        println!(
            " {}[{:>2}] {:<16} ({:>4} MB) {} - {} {}",
            star,
            i + 1,
            model.name,
            model.size_mb,
            lang,
            model.description,
            status
        );
    }

    // --- Parakeet Section ---
    println!("\n--- Parakeet (NVIDIA FastConformer, English) ---\n");

    if parakeet_available {
        for (i, model) in PARAKEET_MODELS.iter().enumerate() {
            let model_path = models_dir.join(model.name);
            let installed = model_path.exists() && validate_parakeet_model(&model_path).is_ok();

            let is_current = is_parakeet_engine && current_parakeet_model == Some(model.name);
            let star = if is_current { "*" } else { " " };

            let status = if installed {
                "\x1b[32m[installed]\x1b[0m"
            } else {
                ""
            };

            println!(
                " {}[{:>2}] {:<28} ({:>4} MB) - {} {}",
                star,
                whisper_count + i + 1,
                model.name,
                model.size_mb,
                model.description,
                status
            );
        }
    } else {
        println!("  \x1b[90m(not available - rebuild with --features parakeet)\x1b[0m");
    }

    // --- Moonshine Section ---
    let moonshine_offset = whisper_count
        + if parakeet_available {
            parakeet_count
        } else {
            0
        };
    println!("\n--- Moonshine (Moonshine AI, encoder-decoder ASR) ---\n");

    if moonshine_available {
        for (i, model) in MOONSHINE_MODELS.iter().enumerate() {
            let model_path = models_dir.join(model.dir_name);
            let installed = model_path.exists() && validate_moonshine_model(&model_path).is_ok();

            let is_current = is_moonshine_engine && current_moonshine_model == Some(model.name);
            let star = if is_current { "*" } else { " " };

            let status = if installed {
                "\x1b[32m[installed]\x1b[0m"
            } else {
                ""
            };

            let license_tag = if model.license == "Community" {
                " \x1b[33m[non-commercial]\x1b[0m"
            } else {
                ""
            };

            println!(
                " {}[{:>2}] {:<20} ({:>4} MB) {} - {}{} {}",
                star,
                moonshine_offset + i + 1,
                model.dir_name,
                model.size_mb,
                model.language,
                model.description,
                license_tag,
                status
            );
        }
    } else {
        println!("  \x1b[90m(not available - rebuild with --features moonshine)\x1b[0m");
    }

    // --- SenseVoice Section ---
    let sensevoice_offset = moonshine_offset
        + if moonshine_available {
            moonshine_count
        } else {
            0
        };
    println!("\n--- SenseVoice (Alibaba FunAudioLLM, CJK + English) ---\n");

    if sensevoice_available {
        for (i, model) in SENSEVOICE_MODELS.iter().enumerate() {
            let model_path = models_dir.join(model.dir_name);
            let installed = model_path.exists() && validate_sensevoice_model(&model_path).is_ok();

            let is_current = is_sensevoice_engine && current_sensevoice_model == Some(model.name);
            let star = if is_current { "*" } else { " " };

            let status = if installed {
                "\x1b[32m[installed]\x1b[0m"
            } else {
                ""
            };

            println!(
                " {}[{:>2}] {:<20} ({:>4} MB) {} - {} {}",
                star,
                sensevoice_offset + i + 1,
                model.dir_name,
                model.size_mb,
                model.languages,
                model.description,
                status
            );
        }
    } else {
        println!("  \x1b[90m(not available - rebuild with --features sensevoice)\x1b[0m");
    }

    // --- Paraformer Section ---
    let paraformer_offset = sensevoice_offset
        + available_count(sensevoice_available, sensevoice_count);
    println!("\n--- Paraformer (FunASR, Chinese + English) ---\n");

    if paraformer_available {
        for (i, model) in PARAFORMER_MODELS.iter().enumerate() {
            let model_path = models_dir.join(model.dir_name);
            let installed = model_path.exists() && validate_onnx_ctc_model(&model_path).is_ok();

            let is_current = is_paraformer_engine && current_paraformer_model == Some(model.name);
            let star = if is_current { "*" } else { " " };

            let status = if installed {
                "\x1b[32m[installed]\x1b[0m"
            } else {
                ""
            };

            println!(
                " {}[{:>2}] {:<20} ({:>4} MB) {} - {} {}",
                star,
                paraformer_offset + i + 1,
                model.dir_name,
                model.size_mb,
                model.languages,
                model.description,
                status
            );
        }
    } else {
        println!("  \x1b[90m(not available - rebuild with --features paraformer)\x1b[0m");
    }

    // --- Dolphin Section ---
    let dolphin_offset = paraformer_offset
        + available_count(paraformer_available, paraformer_count);
    println!("\n--- Dolphin (dictation-optimized CTC) ---\n");

    if dolphin_available {
        for (i, model) in DOLPHIN_MODELS.iter().enumerate() {
            let model_path = models_dir.join(model.dir_name);
            let installed = model_path.exists() && validate_onnx_ctc_model(&model_path).is_ok();

            let is_current = is_dolphin_engine && current_dolphin_model == Some(model.name);
            let star = if is_current { "*" } else { " " };

            let status = if installed {
                "\x1b[32m[installed]\x1b[0m"
            } else {
                ""
            };

            println!(
                " {}[{:>2}] {:<20} ({:>4} MB) {} - {} {}",
                star,
                dolphin_offset + i + 1,
                model.dir_name,
                model.size_mb,
                model.languages,
                model.description,
                status
            );
        }
    } else {
        println!("  \x1b[90m(not available - rebuild with --features dolphin)\x1b[0m");
    }

    // --- Omnilingual Section ---
    let omnilingual_offset = dolphin_offset
        + available_count(dolphin_available, dolphin_count);
    println!("\n--- Omnilingual (FunASR, 50+ languages) ---\n");

    if omnilingual_available {
        for (i, model) in OMNILINGUAL_MODELS.iter().enumerate() {
            let model_path = models_dir.join(model.dir_name);
            let installed = model_path.exists() && validate_onnx_ctc_model(&model_path).is_ok();

            let is_current = is_omnilingual_engine && current_omnilingual_model == Some(model.name);
            let star = if is_current { "*" } else { " " };

            let status = if installed {
                "\x1b[32m[installed]\x1b[0m"
            } else {
                ""
            };

            println!(
                " {}[{:>2}] {:<20} ({:>4} MB) {} - {} {}",
                star,
                omnilingual_offset + i + 1,
                model.dir_name,
                model.size_mb,
                model.languages,
                model.description,
                status
            );
        }
    } else {
        println!("  \x1b[90m(not available - rebuild with --features omnilingual)\x1b[0m");
    }

    println!("\n  [ 0] Cancel\n");

    // Get user selection
    print!("Select model [0-{}]: ", total_count);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let selection: usize = input.trim().parse().unwrap_or(0);

    if selection == 0 {
        println!("\nCancelled.");
        return Ok(());
    }

    // Route to appropriate handler based on selection
    if selection <= whisper_count {
        handle_whisper_selection(selection).await
    } else if parakeet_available && selection <= whisper_count + parakeet_count {
        let parakeet_index = selection - whisper_count;
        handle_parakeet_selection(parakeet_index).await
    } else if moonshine_available && selection <= moonshine_offset + moonshine_count {
        let moonshine_index = selection - moonshine_offset;
        handle_moonshine_selection(moonshine_index).await
    } else if sensevoice_available && selection <= sensevoice_offset + sensevoice_count {
        let sensevoice_index = selection - sensevoice_offset;
        handle_sensevoice_selection(sensevoice_index).await
    } else if paraformer_available && selection <= paraformer_offset + paraformer_count {
        let idx = selection - paraformer_offset;
        handle_onnx_engine_selection("paraformer", PARAFORMER_MODELS.iter().map(|m| (m.name, m.dir_name, m.size_mb, m.files, m.huggingface_repo)).collect(), idx, validate_onnx_ctc_model).await
    } else if dolphin_available && selection <= dolphin_offset + dolphin_count {
        let idx = selection - dolphin_offset;
        handle_onnx_engine_selection("dolphin", DOLPHIN_MODELS.iter().map(|m| (m.name, m.dir_name, m.size_mb, m.files, m.huggingface_repo)).collect(), idx, validate_onnx_ctc_model).await
    } else if omnilingual_available && selection <= omnilingual_offset + omnilingual_count {
        let idx = selection - omnilingual_offset;
        handle_onnx_engine_selection("omnilingual", OMNILINGUAL_MODELS.iter().map(|m| (m.name, m.dir_name, m.size_mb, m.files, m.huggingface_repo)).collect(), idx, validate_onnx_ctc_model).await
    } else {
        println!("\nInvalid selection.");
        Ok(())
    }
}

/// Handle Whisper model selection (download/config)
async fn handle_whisper_selection(selection: usize) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();

    if selection == 0 || selection > MODELS.len() {
        println!("\nCancelled.");
        return Ok(());
    }

    let model = &MODELS[selection - 1];
    let filename = get_model_filename(model.name);
    let model_path = models_dir.join(&filename);

    // Check if already installed
    if model_path.exists() {
        println!("\nModel '{}' is already installed.\n", model.name);
        println!("  [1] Set as default model (update config)");
        println!("  [2] Re-download");
        println!("  [0] Cancel\n");

        print!("Select option [1]: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();

        match choice {
            "" | "1" => {
                // Set as default without re-downloading
                update_config_model(model.name)?;
                restart_daemon_if_running().await;
                return Ok(());
            }
            "2" => {
                // Continue to download below
            }
            _ => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    }

    // Download the model
    download_model(model.name)?;

    // Update config and restart daemon
    update_config_model(model.name)?;
    restart_daemon_if_running().await;

    Ok(())
}

/// Handle Parakeet model selection (download/config)
async fn handle_parakeet_selection(selection: usize) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();

    if selection == 0 || selection > PARAKEET_MODELS.len() {
        println!("\nCancelled.");
        return Ok(());
    }

    let model = &PARAKEET_MODELS[selection - 1];
    let model_path = models_dir.join(model.name);

    // Check if already installed
    if model_path.exists() && validate_parakeet_model(&model_path).is_ok() {
        println!("\nModel '{}' is already installed.\n", model.name);
        println!("  [1] Set as default model (update config)");
        println!("  [2] Re-download");
        println!("  [0] Cancel\n");

        print!("Select option [1]: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();

        match choice {
            "" | "1" => {
                // Set as default without re-downloading
                update_config_parakeet(model.name)?;
                restart_daemon_if_running().await;
                return Ok(());
            }
            "2" => {
                // Continue to download below
            }
            _ => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    }

    // Download the model
    download_parakeet_model_by_info(model)?;

    // Update config and restart daemon
    update_config_parakeet(model.name)?;
    restart_daemon_if_running().await;

    Ok(())
}

/// Handle Moonshine model selection (download/config)
async fn handle_moonshine_selection(selection: usize) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();

    if selection == 0 || selection > MOONSHINE_MODELS.len() {
        println!("\nCancelled.");
        return Ok(());
    }

    let model = &MOONSHINE_MODELS[selection - 1];
    let model_path = models_dir.join(model.dir_name);

    // Check if already installed
    if model_path.exists() && validate_moonshine_model(&model_path).is_ok() {
        println!("\nModel '{}' is already installed.\n", model.dir_name);
        println!("  [1] Set as default model (update config)");
        println!("  [2] Re-download");
        println!("  [0] Cancel\n");

        print!("Select option [1]: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();

        match choice {
            "" | "1" => {
                // Set as default without re-downloading
                update_config_moonshine(model.name)?;
                restart_daemon_if_running().await;
                return Ok(());
            }
            "2" => {
                // Continue to download below
            }
            _ => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    }

    // Show license warning for non-commercial models
    if model.license == "Community" {
        println!();
        print_warning("This model uses the Moonshine Community License (non-commercial use only).");
        print_info("Commercial use requires a separate license from Moonshine AI.");
        println!();
        print!("Continue? [Y/n]: ");
        io::stdout().flush()?;

        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;
        let confirm = confirm.trim().to_lowercase();
        if confirm == "n" || confirm == "no" {
            println!("Cancelled.");
            return Ok(());
        }
    }

    // Download the model
    download_moonshine_model_by_info(model)?;

    // Update config and restart daemon
    update_config_moonshine(model.name)?;
    restart_daemon_if_running().await;

    Ok(())
}

/// Restart the voxtype daemon if it's running
async fn restart_daemon_if_running() {
    // Check if daemon is running via systemd
    let status = tokio::process::Command::new("systemctl")
        .args(["--user", "is-active", "--quiet", "voxtype"])
        .status()
        .await;

    if status.map(|s| s.success()).unwrap_or(false) {
        // Daemon is running, restart it
        println!("\nRestarting voxtype daemon...");
        let restart = tokio::process::Command::new("systemctl")
            .args(["--user", "restart", "voxtype"])
            .status()
            .await;

        match restart {
            Ok(s) if s.success() => {
                print_success("Daemon restarted with new model");
            }
            _ => {
                print_warning("Could not restart daemon");
                print_info("Restart manually: systemctl --user restart voxtype");
            }
        }
    } else {
        println!("\n---");
        println!("Model setup complete!");
    }
}

// =============================================================================
// Whisper Download Functions
// =============================================================================

/// Download a specific Whisper model using curl
pub fn download_model(model_name: &str) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let filename = get_model_filename(model_name);
    let model_path = models_dir.join(&filename);

    // Ensure directory exists
    std::fs::create_dir_all(&models_dir)?;

    let url = get_model_url(model_name);

    println!("\nDownloading {}...", model_name);
    println!("URL: {}", url);

    // Use curl for downloading - it handles progress display and redirects
    let status = Command::new("curl")
        .args([
            "-L",             // Follow redirects
            "--progress-bar", // Show progress bar
            "-o",
            model_path.to_str().unwrap_or("model.bin"),
            &url,
        ])
        .status();

    match status {
        Ok(exit_status) if exit_status.success() => {
            print_success(&format!("Saved to {:?}", model_path));
            Ok(())
        }
        Ok(exit_status) => {
            print_failure(&format!(
                "Download failed: curl exited with code {}",
                exit_status.code().unwrap_or(-1)
            ));
            // Clean up partial download
            let _ = std::fs::remove_file(&model_path);
            anyhow::bail!("Download failed")
        }
        Err(e) => {
            print_failure(&format!("Failed to run curl: {}", e));
            print_info("Please ensure curl is installed (e.g., 'sudo pacman -S curl')");
            anyhow::bail!("curl not available: {}", e)
        }
    }
}

/// Set a specific model as the default (must already be downloaded)
pub async fn set_model(model_name: &str, restart: bool) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let filename = get_model_filename(model_name);
    let model_path = models_dir.join(&filename);

    // Verify the model exists
    if !model_path.exists() {
        print_failure(&format!("Model '{}' is not installed", model_name));
        println!("\n  Run 'voxtype setup model' to download it first.");
        println!("  Or 'voxtype setup model --list' to see installed models.");
        anyhow::bail!("Model not installed: {}", model_name);
    }

    // Update the config
    update_config_model(model_name)?;

    if restart {
        println!("  Restarting daemon...");
        let status = tokio::process::Command::new("systemctl")
            .args(["--user", "restart", "voxtype"])
            .status()
            .await;

        match status {
            Ok(s) if s.success() => {
                print_success("Daemon restarted with new model");
            }
            _ => {
                print_warning("Could not restart daemon (not running as systemd service?)");
                print_info("Restart manually: systemctl --user restart voxtype");
            }
        }
    } else {
        print_info("Restart daemon to use new model: systemctl --user restart voxtype");
        println!(
            "       Or use: voxtype setup model --set {} --restart",
            model_name
        );
    }

    Ok(())
}

/// List installed models
pub fn list_installed() {
    println!("Installed Whisper Models\n");
    println!("========================\n");

    let models_dir = Config::models_dir();

    if !models_dir.exists() {
        println!("No models directory found: {:?}", models_dir);
        return;
    }

    let mut found = false;

    for model in MODELS {
        let filename = get_model_filename(model.name);
        let model_path = models_dir.join(&filename);

        if model_path.exists() {
            let size = std::fs::metadata(&model_path)
                .map(|m| m.len() as f64 / 1024.0 / 1024.0)
                .unwrap_or(0.0);

            println!("  {} ({:.0} MB) - {}", model.name, size, model.description);
            found = true;
        }
    }

    if !found {
        println!("  No models installed.");
        println!("\n  Run 'voxtype setup model' to download a model.");
    }
}

/// Update the config file to use a specific model (with status messages)
fn update_config_model(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_model_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
            print_success(&format!("Config updated to use '{}' model", model_name));
            Ok(())
        } else {
            print_info("No config file found. Run 'voxtype setup' first.");
            Ok(())
        }
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update the config file to use a specific model (quiet, no output)
pub fn set_model_config(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_model_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
        }
        // Silently succeed if config doesn't exist yet - setup will create it
        Ok(())
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update the model setting in a config string (also sets engine to whisper)
fn update_model_in_config(config: &str, model_name: &str) -> String {
    // Simple regex-free replacement for the model line
    let mut result = String::new();
    let mut in_whisper_section = false;
    let mut engine_updated = false;

    for line in config.lines() {
        let trimmed = line.trim();

        // Track if we're in a section
        if trimmed.starts_with('[') {
            in_whisper_section = trimmed == "[whisper]";
        }

        // Update engine line to whisper (at top level, before any section)
        if trimmed.starts_with("engine") && !trimmed.starts_with('[') {
            result.push_str("engine = \"whisper\"\n");
            engine_updated = true;
        }
        // Replace model line in whisper section
        else if in_whisper_section && trimmed.starts_with("model") {
            result.push_str(&format!("model = \"{}\"\n", model_name));
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // If no engine line existed, we don't need to add one (whisper is the default)
    // But if engine was set to something else, we've already updated it above
    let _ = engine_updated; // suppress unused warning

    // Remove trailing newline if original didn't have one
    if !config.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

// =============================================================================
// Parakeet Model Functions
// =============================================================================

/// Check if a model name is a Parakeet model
pub fn is_parakeet_model(name: &str) -> bool {
    PARAKEET_MODELS.iter().any(|m| m.name == name)
}

/// Get list of valid Parakeet model names
pub fn valid_parakeet_model_names() -> Vec<&'static str> {
    PARAKEET_MODELS.iter().map(|m| m.name).collect()
}

/// Validate that a Parakeet model directory has the required files
pub fn validate_parakeet_model(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("Model directory does not exist: {:?}", path);
    }

    // Check for TDT structure: encoder + decoder + vocab
    let has_encoder = path.join("encoder-model.onnx").exists()
        || path.join("encoder-model.onnx.data").exists()
        || path.join("encoder-model.int8.onnx").exists();
    let has_decoder = path.join("decoder_joint-model.onnx").exists()
        || path.join("decoder_joint-model.int8.onnx").exists();
    let has_vocab = path.join("vocab.txt").exists();

    if has_encoder && has_decoder && has_vocab {
        Ok(())
    } else {
        let mut missing = Vec::new();
        if !has_encoder {
            missing.push("encoder model");
        }
        if !has_decoder {
            missing.push("decoder model");
        }
        if !has_vocab {
            missing.push("vocab.txt");
        }
        anyhow::bail!("Incomplete Parakeet model, missing: {}", missing.join(", "))
    }
}

/// Download a Parakeet model by name (public API for run_setup)
pub fn download_parakeet_model(model_name: &str) -> anyhow::Result<()> {
    let model = PARAKEET_MODELS
        .iter()
        .find(|m| m.name == model_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown Parakeet model: {}", model_name))?;

    download_parakeet_model_by_info(model)
}

/// Download a Parakeet model using its info struct
fn download_parakeet_model_by_info(model: &ParakeetModelInfo) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let model_path = models_dir.join(model.name);

    // Create model directory
    std::fs::create_dir_all(&model_path)?;

    println!("\nDownloading {} ({} MB)...\n", model.name, model.size_mb);

    for (filename, _expected_size) in model.files {
        let file_path = model_path.join(filename);

        if file_path.exists() {
            println!("  {} already exists, skipping", filename);
            continue;
        }

        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            model.huggingface_repo, filename
        );

        println!("Downloading {}...", filename);

        let status = Command::new("curl")
            .args([
                "-L",
                "--progress-bar",
                "-o",
                file_path.to_str().unwrap_or("file"),
                &url,
            ])
            .status();

        match status {
            Ok(exit_status) if exit_status.success() => {
                // Success, continue
            }
            Ok(exit_status) => {
                print_failure(&format!(
                    "Download failed: curl exited with code {}",
                    exit_status.code().unwrap_or(-1)
                ));
                // Clean up partial download
                let _ = std::fs::remove_file(&file_path);
                anyhow::bail!("Download failed for {}", filename)
            }
            Err(e) => {
                print_failure(&format!("Failed to run curl: {}", e));
                print_info("Please ensure curl is installed (e.g., 'sudo pacman -S curl')");
                anyhow::bail!("curl not available: {}", e)
            }
        }
    }

    // Validate all files are present
    validate_parakeet_model(&model_path)?;
    print_success(&format!(
        "Model '{}' downloaded to {:?}",
        model.name, model_path
    ));

    Ok(())
}

/// Update config to use Parakeet engine and a specific model (with status messages)
fn update_config_parakeet(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_parakeet_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
            print_success(&format!(
                "Config updated: engine = \"parakeet\", model = \"{}\"",
                model_name
            ));
            Ok(())
        } else {
            print_info("No config file found. Run 'voxtype setup' first.");
            Ok(())
        }
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update config to use Parakeet engine and a specific model (quiet, no output)
pub fn set_parakeet_config(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_parakeet_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
        }
        Ok(())
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update the config to use Parakeet engine with a specific model
fn update_parakeet_in_config(config: &str, model_name: &str) -> String {
    let mut result = String::new();
    let mut has_engine_line = false;
    let mut has_parakeet_section = false;
    let mut in_parakeet_section = false;
    let mut parakeet_model_updated = false;

    for line in config.lines() {
        let trimmed = line.trim();

        // Track sections
        if trimmed.starts_with('[') {
            // If we were in parakeet section and didn't update model, add it
            if in_parakeet_section && !parakeet_model_updated {
                result.push_str(&format!("model = \"{}\"\n", model_name));
                parakeet_model_updated = true;
            }
            in_parakeet_section = trimmed == "[parakeet]";
            if in_parakeet_section {
                has_parakeet_section = true;
            }
        }

        // Update or add engine line at the top level
        if trimmed.starts_with("engine") && !trimmed.starts_with('[') {
            result.push_str("engine = \"parakeet\"\n");
            has_engine_line = true;
        }
        // Update model line in parakeet section
        else if in_parakeet_section && trimmed.starts_with("model") {
            result.push_str(&format!("model = \"{}\"\n", model_name));
            parakeet_model_updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // If we were in parakeet section at EOF and didn't update model, add it
    if in_parakeet_section && !parakeet_model_updated {
        result.push_str(&format!("model = \"{}\"\n", model_name));
    }

    // Add engine line if not present (at the very beginning after any comments)
    if !has_engine_line {
        // Find first non-comment, non-empty line or section
        let mut new_result = String::new();
        let mut engine_added = false;
        for line in result.lines() {
            let trimmed = line.trim();
            if !engine_added
                && !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.starts_with("engine")
            {
                new_result.push_str("engine = \"parakeet\"\n\n");
                engine_added = true;
            }
            new_result.push_str(line);
            new_result.push('\n');
        }
        result = new_result;
    }

    // Add [parakeet] section if not present
    if !has_parakeet_section {
        result.push_str(&format!("\n[parakeet]\nmodel = \"{}\"\n", model_name));
    }

    // Remove trailing newline if original didn't have one
    if !config.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// List installed Parakeet models
pub fn list_installed_parakeet() {
    println!("\nInstalled Parakeet Models (EXPERIMENTAL)\n");
    println!("=========================================\n");

    let models_dir = Config::models_dir();

    if !models_dir.exists() {
        println!("No models directory found: {:?}", models_dir);
        return;
    }

    let mut found = false;

    for model in PARAKEET_MODELS {
        let model_path = models_dir.join(model.name);

        if model_path.exists() && validate_parakeet_model(&model_path).is_ok() {
            let size = std::fs::read_dir(&model_path)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.metadata().ok())
                        .map(|m| m.len() as f64 / 1024.0 / 1024.0)
                        .sum::<f64>()
                })
                .unwrap_or(0.0);

            println!("  {} ({:.0} MB) - {}", model.name, size, model.description);
            found = true;
        }
    }

    if !found {
        println!("  No Parakeet models installed.");
        println!("\n  Run 'voxtype setup model' and select Parakeet to download.");
    }
}

// =============================================================================
// Moonshine Model Functions
// =============================================================================

/// Check if a model name is a Moonshine model
pub fn is_moonshine_model(name: &str) -> bool {
    MOONSHINE_MODELS.iter().any(|m| m.name == name)
}

/// Get list of valid Moonshine model names
pub fn valid_moonshine_model_names() -> Vec<&'static str> {
    MOONSHINE_MODELS.iter().map(|m| m.name).collect()
}

/// Validate that a Moonshine model directory has the required files
pub fn validate_moonshine_model(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("Model directory does not exist: {:?}", path);
    }

    let has_encoder = path.join("encoder_model.onnx").exists();
    let has_decoder = path.join("decoder_model_merged.onnx").exists();
    let has_tokenizer = path.join("tokenizer.json").exists();

    if has_encoder && has_decoder && has_tokenizer {
        Ok(())
    } else {
        let mut missing = Vec::new();
        if !has_encoder {
            missing.push("encoder_model.onnx");
        }
        if !has_decoder {
            missing.push("decoder_model_merged.onnx");
        }
        if !has_tokenizer {
            missing.push("tokenizer.json");
        }
        anyhow::bail!(
            "Incomplete Moonshine model, missing: {}",
            missing.join(", ")
        )
    }
}

/// Download a Moonshine model by name (public API for run_setup)
pub fn download_moonshine_model(model_name: &str) -> anyhow::Result<()> {
    let model = MOONSHINE_MODELS
        .iter()
        .find(|m| m.name == model_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown Moonshine model: {}", model_name))?;

    download_moonshine_model_by_info(model)
}

/// Download a Moonshine model using its info struct
fn download_moonshine_model_by_info(model: &MoonshineModelInfo) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let model_path = models_dir.join(model.dir_name);

    // Create model directory
    std::fs::create_dir_all(&model_path)?;

    println!(
        "\nDownloading {} ({} MB)...\n",
        model.dir_name, model.size_mb
    );

    for (repo_path, local_filename) in model.files {
        let file_path = model_path.join(local_filename);

        if file_path.exists() {
            println!("  {} already exists, skipping", local_filename);
            continue;
        }

        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            model.huggingface_repo, repo_path
        );

        println!("Downloading {}...", local_filename);

        let status = Command::new("curl")
            .args([
                "-L",
                "--progress-bar",
                "-o",
                file_path.to_str().unwrap_or("file"),
                &url,
            ])
            .status();

        match status {
            Ok(exit_status) if exit_status.success() => {
                // Success, continue
            }
            Ok(exit_status) => {
                print_failure(&format!(
                    "Download failed: curl exited with code {}",
                    exit_status.code().unwrap_or(-1)
                ));
                // Clean up partial download
                let _ = std::fs::remove_file(&file_path);
                anyhow::bail!("Download failed for {}", local_filename)
            }
            Err(e) => {
                print_failure(&format!("Failed to run curl: {}", e));
                print_info("Please ensure curl is installed (e.g., 'sudo pacman -S curl')");
                anyhow::bail!("curl not available: {}", e)
            }
        }
    }

    // Validate all files are present
    validate_moonshine_model(&model_path)?;
    print_success(&format!(
        "Model '{}' downloaded to {:?}",
        model.dir_name, model_path
    ));

    Ok(())
}

/// Update config to use Moonshine engine and a specific model (with status messages)
fn update_config_moonshine(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_moonshine_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
            print_success(&format!(
                "Config updated: engine = \"moonshine\", model = \"{}\"",
                model_name
            ));
            Ok(())
        } else {
            print_info("No config file found. Run 'voxtype setup' first.");
            Ok(())
        }
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update the config to use Moonshine engine with a specific model
fn update_moonshine_in_config(config: &str, model_name: &str) -> String {
    let mut result = String::new();
    let mut has_engine_line = false;
    let mut has_moonshine_section = false;
    let mut in_moonshine_section = false;
    let mut moonshine_model_updated = false;

    for line in config.lines() {
        let trimmed = line.trim();

        // Track sections
        if trimmed.starts_with('[') {
            // If we were in moonshine section and didn't update model, add it
            if in_moonshine_section && !moonshine_model_updated {
                result.push_str(&format!("model = \"{}\"\n", model_name));
                moonshine_model_updated = true;
            }
            in_moonshine_section = trimmed == "[moonshine]";
            if in_moonshine_section {
                has_moonshine_section = true;
            }
        }

        // Update or add engine line at the top level
        if trimmed.starts_with("engine") && !trimmed.starts_with('[') {
            result.push_str("engine = \"moonshine\"\n");
            has_engine_line = true;
        }
        // Update model line in moonshine section
        else if in_moonshine_section && trimmed.starts_with("model") {
            result.push_str(&format!("model = \"{}\"\n", model_name));
            moonshine_model_updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // If we were in moonshine section at EOF and didn't update model, add it
    if in_moonshine_section && !moonshine_model_updated {
        result.push_str(&format!("model = \"{}\"\n", model_name));
    }

    // Add engine line if not present
    if !has_engine_line {
        let mut new_result = String::new();
        let mut engine_added = false;
        for line in result.lines() {
            let trimmed = line.trim();
            if !engine_added
                && !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.starts_with("engine")
            {
                new_result.push_str("engine = \"moonshine\"\n\n");
                engine_added = true;
            }
            new_result.push_str(line);
            new_result.push('\n');
        }
        result = new_result;
    }

    // Add [moonshine] section if not present
    if !has_moonshine_section {
        result.push_str(&format!("\n[moonshine]\nmodel = \"{}\"\n", model_name));
    }

    // Remove trailing newline if original didn't have one
    if !config.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// Handle SenseVoice model selection (download/config)
async fn handle_sensevoice_selection(selection: usize) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();

    if selection == 0 || selection > SENSEVOICE_MODELS.len() {
        println!("\nCancelled.");
        return Ok(());
    }

    let model = &SENSEVOICE_MODELS[selection - 1];
    let model_path = models_dir.join(model.dir_name);

    // Check if already installed
    if model_path.exists() && validate_sensevoice_model(&model_path).is_ok() {
        println!("\nModel '{}' is already installed.\n", model.dir_name);
        println!("  [1] Set as default model (update config)");
        println!("  [2] Re-download");
        println!("  [0] Cancel\n");

        print!("Select option [1]: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();

        match choice {
            "" | "1" => {
                update_config_sensevoice(model.name)?;
                restart_daemon_if_running().await;
                return Ok(());
            }
            "2" => {
                // Continue to download below
            }
            _ => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    }

    // Download the model
    download_sensevoice_model_by_info(model)?;

    // Update config and restart daemon
    update_config_sensevoice(model.name)?;
    restart_daemon_if_running().await;

    Ok(())
}

/// List installed Moonshine models
pub fn list_installed_moonshine() {
    println!("\nInstalled Moonshine Models\n");
    println!("==========================\n");

    let models_dir = Config::models_dir();

    if !models_dir.exists() {
        println!("No models directory found: {:?}", models_dir);
        return;
    }

    let mut found = false;

    for model in MOONSHINE_MODELS {
        let model_path = models_dir.join(model.dir_name);

        if model_path.exists() && validate_moonshine_model(&model_path).is_ok() {
            let size = std::fs::read_dir(&model_path)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.metadata().ok())
                        .map(|m| m.len() as f64 / 1024.0 / 1024.0)
                        .sum::<f64>()
                })
                .unwrap_or(0.0);

            let license_note = if model.license == "Community" {
                " [non-commercial]"
            } else {
                ""
            };

            println!(
                "  {} ({:.0} MB) - {} ({}){}",
                model.dir_name, size, model.description, model.language, license_note
            );
            found = true;
        }
    }

    if !found {
        println!("  No Moonshine models installed.");
        println!("\n  Run 'voxtype setup model' and select Moonshine to download.");
    }
}

// =============================================================================
// SenseVoice Model Functions
// =============================================================================

/// Check if a model name is a SenseVoice model
pub fn is_sensevoice_model(name: &str) -> bool {
    SENSEVOICE_MODELS.iter().any(|m| m.name == name)
}

/// Get the directory name for a SenseVoice model
pub fn sensevoice_dir_name(name: &str) -> Option<&'static str> {
    SENSEVOICE_MODELS
        .iter()
        .find(|m| m.name == name)
        .map(|m| m.dir_name)
}

/// Get list of valid SenseVoice model names
pub fn valid_sensevoice_model_names() -> Vec<&'static str> {
    SENSEVOICE_MODELS.iter().map(|m| m.name).collect()
}

/// Validate that a SenseVoice model directory has the required files
pub fn validate_sensevoice_model(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("Model directory does not exist: {:?}", path);
    }

    let has_model =
        path.join("model.int8.onnx").exists() || path.join("model.onnx").exists();
    let has_tokens = path.join("tokens.txt").exists();

    if has_model && has_tokens {
        Ok(())
    } else {
        let mut missing = Vec::new();
        if !has_model {
            missing.push("model.int8.onnx or model.onnx");
        }
        if !has_tokens {
            missing.push("tokens.txt");
        }
        anyhow::bail!(
            "Incomplete SenseVoice model, missing: {}",
            missing.join(", ")
        )
    }
}

/// Download a SenseVoice model by name (public API for run_setup)
pub fn download_sensevoice_model(model_name: &str) -> anyhow::Result<()> {
    let model = SENSEVOICE_MODELS
        .iter()
        .find(|m| m.name == model_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown SenseVoice model: {}", model_name))?;

    download_sensevoice_model_by_info(model)
}

/// Download a SenseVoice model using its info struct
fn download_sensevoice_model_by_info(model: &SenseVoiceModelInfo) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let model_path = models_dir.join(model.dir_name);

    // Create model directory
    std::fs::create_dir_all(&model_path)?;

    println!(
        "\nDownloading {} ({} MB)...\n",
        model.dir_name, model.size_mb
    );

    for (repo_path, local_filename) in model.files {
        let file_path = model_path.join(local_filename);

        if file_path.exists() {
            println!("  {} already exists, skipping", local_filename);
            continue;
        }

        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            model.huggingface_repo, repo_path
        );

        println!("Downloading {}...", local_filename);

        let status = Command::new("curl")
            .args([
                "-L",
                "--progress-bar",
                "-o",
                file_path.to_str().unwrap_or("file"),
                &url,
            ])
            .status();

        match status {
            Ok(exit_status) if exit_status.success() => {
                // Success, continue
            }
            Ok(exit_status) => {
                print_failure(&format!(
                    "Download failed: curl exited with code {}",
                    exit_status.code().unwrap_or(-1)
                ));
                let _ = std::fs::remove_file(&file_path);
                anyhow::bail!("Download failed for {}", local_filename)
            }
            Err(e) => {
                print_failure(&format!("Failed to run curl: {}", e));
                print_info("Please ensure curl is installed (e.g., 'sudo pacman -S curl')");
                anyhow::bail!("curl not available: {}", e)
            }
        }
    }

    // Validate all files are present
    validate_sensevoice_model(&model_path)?;
    print_success(&format!(
        "Model '{}' downloaded to {:?}",
        model.dir_name, model_path
    ));

    Ok(())
}

/// Update config to use SenseVoice engine and a specific model (with status messages)
fn update_config_sensevoice(model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_sensevoice_in_config(&content, model_name);
            std::fs::write(&config_path, updated)?;
            print_success(&format!(
                "Config updated: engine = \"sensevoice\", model = \"{}\"",
                model_name
            ));
            Ok(())
        } else {
            print_info("No config file found. Run 'voxtype setup' first.");
            Ok(())
        }
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update the config to use SenseVoice engine with a specific model
fn update_sensevoice_in_config(config: &str, model_name: &str) -> String {
    let mut result = String::new();
    let mut has_engine_line = false;
    let mut has_sensevoice_section = false;
    let mut in_sensevoice_section = false;
    let mut sensevoice_model_updated = false;

    for line in config.lines() {
        let trimmed = line.trim();

        // Track sections
        if trimmed.starts_with('[') {
            if in_sensevoice_section && !sensevoice_model_updated {
                result.push_str(&format!("model = \"{}\"\n", model_name));
                sensevoice_model_updated = true;
            }
            in_sensevoice_section = trimmed == "[sensevoice]";
            if in_sensevoice_section {
                has_sensevoice_section = true;
            }
        }

        // Update or add engine line at the top level
        if trimmed.starts_with("engine") && !trimmed.starts_with('[') {
            result.push_str("engine = \"sensevoice\"\n");
            has_engine_line = true;
        }
        // Update model line in sensevoice section
        else if in_sensevoice_section && trimmed.starts_with("model") {
            result.push_str(&format!("model = \"{}\"\n", model_name));
            sensevoice_model_updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    // If we were in sensevoice section at EOF and didn't update model, add it
    if in_sensevoice_section && !sensevoice_model_updated {
        result.push_str(&format!("model = \"{}\"\n", model_name));
    }

    // Add engine line if not present
    if !has_engine_line {
        let mut new_result = String::new();
        let mut engine_added = false;
        for line in result.lines() {
            let trimmed = line.trim();
            if !engine_added
                && !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.starts_with("engine")
            {
                new_result.push_str("engine = \"sensevoice\"\n\n");
                engine_added = true;
            }
            new_result.push_str(line);
            new_result.push('\n');
        }
        result = new_result;
    }

    // Add [sensevoice] section if not present
    if !has_sensevoice_section {
        result.push_str(&format!("\n[sensevoice]\nmodel = \"{}\"\n", model_name));
    }

    // Remove trailing newline if original didn't have one
    if !config.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

/// List installed SenseVoice models
pub fn list_installed_sensevoice() {
    println!("\nInstalled SenseVoice Models\n");
    println!("===========================\n");

    let models_dir = Config::models_dir();

    if !models_dir.exists() {
        println!("No models directory found: {:?}", models_dir);
        return;
    }

    let mut found = false;

    for model in SENSEVOICE_MODELS {
        let model_path = models_dir.join(model.dir_name);

        if model_path.exists() && validate_sensevoice_model(&model_path).is_ok() {
            let size = std::fs::read_dir(&model_path)
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.metadata().ok())
                        .map(|m| m.len() as f64 / 1024.0 / 1024.0)
                        .sum::<f64>()
                })
                .unwrap_or(0.0);

            println!(
                "  {} ({:.0} MB) - {} ({})",
                model.dir_name, size, model.description, model.languages
            );
            found = true;
        }
    }

    if !found {
        println!("  No SenseVoice models installed.");
        println!("\n  Run 'voxtype setup model' and select SenseVoice to download.");
    }
}

// =============================================================================
// Generic ONNX Engine Functions (Paraformer, Dolphin, Omnilingual)
// =============================================================================

/// Validate a CTC-based ONNX model directory (model.int8.onnx or model.onnx + tokens.txt)
fn validate_onnx_ctc_model(path: &Path) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("Model directory does not exist: {:?}", path);
    }

    let has_model = path.join("model.int8.onnx").exists() || path.join("model.onnx").exists();
    let has_tokens = path.join("tokens.txt").exists();

    if has_model && has_tokens {
        Ok(())
    } else {
        let mut missing = Vec::new();
        if !has_model {
            missing.push("model.int8.onnx or model.onnx");
        }
        if !has_tokens {
            missing.push("tokens.txt");
        }
        anyhow::bail!("Incomplete model, missing: {}", missing.join(", "))
    }
}

/// Generic handler for ONNX engine model selection (download/config/restart)
async fn handle_onnx_engine_selection(
    engine_name: &str,
    models: Vec<(&str, &str, u32, &[(&str, &str)], &str)>,
    selection: usize,
    validate_fn: fn(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();

    if selection == 0 || selection > models.len() {
        println!("\nCancelled.");
        return Ok(());
    }

    let (name, dir_name, size_mb, files, repo) = &models[selection - 1];
    let model_path = models_dir.join(dir_name);

    // Check if already installed
    if model_path.exists() && validate_fn(&model_path).is_ok() {
        println!("\nModel '{}' is already installed.\n", dir_name);
        println!("  [1] Set as default model (update config)");
        println!("  [2] Re-download");
        println!("  [0] Cancel\n");

        print!("Select option [1]: ");
        io::stdout().flush()?;

        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();

        match choice {
            "" | "1" => {
                update_config_engine(engine_name, name)?;
                restart_daemon_if_running().await;
                return Ok(());
            }
            "2" => {
                // Continue to download below
            }
            _ => {
                println!("Cancelled.");
                return Ok(());
            }
        }
    }

    // Download the model
    download_onnx_model(dir_name, *size_mb, files, repo)?;

    // Validate
    validate_fn(&model_path)?;
    print_success(&format!("Model '{}' downloaded to {:?}", dir_name, model_path));

    // Update config and restart daemon
    update_config_engine(engine_name, name)?;
    restart_daemon_if_running().await;

    Ok(())
}

/// Download an ONNX model from HuggingFace
fn download_onnx_model(
    dir_name: &str,
    size_mb: u32,
    files: &[(&str, &str)],
    repo: &str,
) -> anyhow::Result<()> {
    let models_dir = Config::models_dir();
    let model_path = models_dir.join(dir_name);

    std::fs::create_dir_all(&model_path)?;

    println!("\nDownloading {} ({} MB)...\n", dir_name, size_mb);

    for (repo_path, local_filename) in files {
        let file_path = model_path.join(local_filename);

        if file_path.exists() {
            println!("  {} already exists, skipping", local_filename);
            continue;
        }

        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            repo, repo_path
        );

        println!("Downloading {}...", local_filename);

        let status = Command::new("curl")
            .args([
                "-L",
                "--progress-bar",
                "-o",
                file_path.to_str().unwrap_or("file"),
                &url,
            ])
            .status();

        match status {
            Ok(exit_status) if exit_status.success() => {}
            Ok(exit_status) => {
                print_failure(&format!(
                    "Download failed: curl exited with code {}",
                    exit_status.code().unwrap_or(-1)
                ));
                let _ = std::fs::remove_file(&file_path);
                anyhow::bail!("Download failed for {}", local_filename)
            }
            Err(e) => {
                print_failure(&format!("Failed to run curl: {}", e));
                print_info("Please ensure curl is installed (e.g., 'sudo pacman -S curl')");
                anyhow::bail!("curl not available: {}", e)
            }
        }
    }

    Ok(())
}

/// Update config to use a specific engine and model
fn update_config_engine(engine_name: &str, model_name: &str) -> anyhow::Result<()> {
    if let Some(config_path) = Config::default_path() {
        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)?;
            let updated = update_engine_in_config(&content, engine_name, model_name);
            std::fs::write(&config_path, updated)?;
            print_success(&format!(
                "Config updated: engine = \"{}\", model = \"{}\"",
                engine_name, model_name
            ));
            Ok(())
        } else {
            print_info("No config file found. Run 'voxtype setup' first.");
            Ok(())
        }
    } else {
        anyhow::bail!("Could not determine config path")
    }
}

/// Update a config string to use a specific engine and model
fn update_engine_in_config(config: &str, engine_name: &str, model_name: &str) -> String {
    let section_name = format!("[{}]", engine_name);
    let mut result = String::new();
    let mut has_engine_line = false;
    let mut has_section = false;
    let mut in_section = false;
    let mut model_updated = false;

    for line in config.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('[') {
            if in_section && !model_updated {
                result.push_str(&format!("model = \"{}\"\n", model_name));
                model_updated = true;
            }
            in_section = trimmed == section_name;
            if in_section {
                has_section = true;
            }
        }

        if trimmed.starts_with("engine") && !trimmed.starts_with('[') {
            result.push_str(&format!("engine = \"{}\"\n", engine_name));
            has_engine_line = true;
        } else if in_section && trimmed.starts_with("model") {
            result.push_str(&format!("model = \"{}\"\n", model_name));
            model_updated = true;
        } else {
            result.push_str(line);
            result.push('\n');
        }
    }

    if in_section && !model_updated {
        result.push_str(&format!("model = \"{}\"\n", model_name));
    }

    if !has_engine_line {
        let mut new_result = String::new();
        let mut engine_added = false;
        for line in result.lines() {
            let trimmed = line.trim();
            if !engine_added
                && !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.starts_with("engine")
            {
                new_result.push_str(&format!("engine = \"{}\"\n\n", engine_name));
                engine_added = true;
            }
            new_result.push_str(line);
            new_result.push('\n');
        }
        result = new_result;
    }

    if !has_section {
        result.push_str(&format!("\n[{}]\nmodel = \"{}\"\n", engine_name, model_name));
    }

    if !config.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_update_model_in_config_basic() {
        let config = r#"[whisper]
model = "base.en"
language = "en"
"#;
        let result = update_model_in_config(config, "large-v3");
        assert!(result.contains(r#"model = "large-v3""#));
        assert!(!result.contains("base.en"));
    }

    #[test]
    fn test_update_model_in_config_switches_engine_to_whisper() {
        // When switching to a Whisper model, engine should be set to whisper
        let config = r#"engine = "parakeet"

[whisper]
model = "small"

[parakeet]
model = "parakeet-tdt-0.6b-v3"
"#;
        let result = update_model_in_config(config, "base.en");
        // Engine should now be whisper
        assert!(result.contains(r#"engine = "whisper""#));
        assert!(!result.contains(r#"engine = "parakeet""#));
        // Whisper model should be updated
        assert!(result.contains(r#"model = "base.en""#));
        // Parakeet section should be preserved
        assert!(result.contains("[parakeet]"));
        assert!(result.contains(r#"model = "parakeet-tdt-0.6b-v3""#));
    }

    #[test]
    fn test_update_model_in_config_preserves_other_sections() {
        let config = r#"[hotkey]
key = "SCROLLLOCK"

[whisper]
model = "base.en"
language = "en"

[output]
mode = "type"
"#;
        let result = update_model_in_config(config, "small.en");
        assert!(result.contains(r#"model = "small.en""#));
        assert!(result.contains(r#"key = "SCROLLLOCK""#));
        assert!(result.contains(r#"mode = "type""#));
        assert!(result.contains("[hotkey]"));
        assert!(result.contains("[output]"));
    }

    #[test]
    fn test_update_model_in_config_only_changes_whisper_section() {
        // If there's a "model" key in another section, it should not be changed
        let config = r#"[some_other_section]
model = "should_not_change"

[whisper]
model = "base.en"
"#;
        let result = update_model_in_config(config, "large-v3");
        assert!(result.contains(r#"model = "should_not_change""#));
        assert!(result.contains(r#"model = "large-v3""#));
    }

    #[test]
    fn test_update_model_in_config_handles_comments() {
        let config = r#"[whisper]
# Model to use
model = "base.en"
# Language setting
language = "en"
"#;
        let result = update_model_in_config(config, "medium.en");
        assert!(result.contains(r#"model = "medium.en""#));
        assert!(result.contains("# Model to use"));
        assert!(result.contains("# Language setting"));
    }

    #[test]
    fn test_models_list_contains_expected_models() {
        let model_names: Vec<&str> = MODELS.iter().map(|m| m.name).collect();
        // Multilingual models
        assert!(model_names.contains(&"tiny"));
        assert!(model_names.contains(&"base"));
        assert!(model_names.contains(&"small"));
        assert!(model_names.contains(&"medium"));
        // English-only models
        assert!(model_names.contains(&"tiny.en"));
        assert!(model_names.contains(&"base.en"));
        assert!(model_names.contains(&"small.en"));
        assert!(model_names.contains(&"medium.en"));
        // Large models (multilingual only)
        assert!(model_names.contains(&"large-v3"));
        assert!(model_names.contains(&"large-v3-turbo"));
    }

    #[test]
    fn test_model_info_sizes_are_reasonable() {
        for model in MODELS {
            // All models should have positive size
            assert!(model.size_mb > 0, "Model {} has invalid size", model.name);
            // Tiny models should be smallest, large should be biggest
            if model.name.starts_with("tiny") {
                assert!(model.size_mb < 100);
            }
            if model.name == "large-v3" {
                assert!(model.size_mb > 2000);
            }
        }
    }

    #[test]
    fn test_is_valid_model() {
        // Valid multilingual models
        assert!(is_valid_model("tiny"));
        assert!(is_valid_model("base"));
        assert!(is_valid_model("small"));
        assert!(is_valid_model("medium"));
        // Valid English-only models
        assert!(is_valid_model("tiny.en"));
        assert!(is_valid_model("base.en"));
        assert!(is_valid_model("small.en"));
        assert!(is_valid_model("medium.en"));
        // Valid large models
        assert!(is_valid_model("large-v3"));
        assert!(is_valid_model("large-v3-turbo"));

        // Invalid models
        assert!(!is_valid_model("invalid"));
        assert!(!is_valid_model("large"));
        assert!(!is_valid_model(""));
        assert!(!is_valid_model("LARGE-V3")); // case sensitive
    }

    #[test]
    fn test_valid_model_names() {
        let names = valid_model_names();
        assert!(names.contains(&"tiny.en"));
        assert!(names.contains(&"large-v3-turbo"));
        assert_eq!(names.len(), MODELS.len());
    }

    // =========================================================================
    // Parakeet Model Tests
    // =========================================================================

    #[test]
    fn test_parakeet_models_list_contains_expected_models() {
        let model_names: Vec<&str> = PARAKEET_MODELS.iter().map(|m| m.name).collect();
        assert!(model_names.contains(&"parakeet-tdt-0.6b-v3"));
        assert!(model_names.contains(&"parakeet-tdt-0.6b-v3-int8"));
    }

    #[test]
    fn test_parakeet_model_info_sizes_are_reasonable() {
        for model in PARAKEET_MODELS {
            // All models should have positive size
            assert!(model.size_mb > 0, "Model {} has invalid size", model.name);
            // Full model should be larger than quantized
            if model.name == "parakeet-tdt-0.6b-v3" {
                assert!(model.size_mb > 2000);
            }
            if model.name == "parakeet-tdt-0.6b-v3-int8" {
                assert!(model.size_mb < 1000);
            }
        }
    }

    #[test]
    fn test_parakeet_models_have_files() {
        for model in PARAKEET_MODELS {
            assert!(
                !model.files.is_empty(),
                "Model {} should have file definitions",
                model.name
            );
            // All TDT models should have vocab.txt
            assert!(
                model.files.iter().any(|(f, _)| *f == "vocab.txt"),
                "Model {} should have vocab.txt",
                model.name
            );
        }
    }

    #[test]
    fn test_is_parakeet_model() {
        // Valid Parakeet models
        assert!(is_parakeet_model("parakeet-tdt-0.6b-v3"));
        assert!(is_parakeet_model("parakeet-tdt-0.6b-v3-int8"));

        // Invalid models
        assert!(!is_parakeet_model("base.en"));
        assert!(!is_parakeet_model("large-v3"));
        assert!(!is_parakeet_model("parakeet")); // Not a full model name
        assert!(!is_parakeet_model(""));
    }

    #[test]
    fn test_valid_parakeet_model_names() {
        let names = valid_parakeet_model_names();
        assert!(names.contains(&"parakeet-tdt-0.6b-v3"));
        assert!(names.contains(&"parakeet-tdt-0.6b-v3-int8"));
        assert_eq!(names.len(), PARAKEET_MODELS.len());
    }

    #[test]
    fn test_update_parakeet_in_config_basic() {
        let config = r#"[hotkey]
key = "SCROLLLOCK"

[whisper]
model = "base.en"
language = "en"

[output]
mode = "type"
"#;
        let result = update_parakeet_in_config(config, "parakeet-tdt-0.6b-v3");

        // Should add engine = "parakeet"
        assert!(result.contains(r#"engine = "parakeet""#));
        // Should add [parakeet] section with model
        assert!(result.contains("[parakeet]"));
        assert!(result.contains(r#"model = "parakeet-tdt-0.6b-v3""#));
        // Should preserve existing sections
        assert!(result.contains("[whisper]"));
        assert!(result.contains("[hotkey]"));
        assert!(result.contains("[output]"));
    }

    #[test]
    fn test_update_parakeet_in_config_updates_existing() {
        let config = r#"engine = "whisper"

[hotkey]
key = "SCROLLLOCK"

[whisper]
model = "base.en"
language = "en"

[parakeet]
model = "old-model"

[output]
mode = "type"
"#;
        let result = update_parakeet_in_config(config, "parakeet-tdt-0.6b-v3-int8");

        // Should update engine to parakeet
        assert!(result.contains(r#"engine = "parakeet""#));
        assert!(!result.contains(r#"engine = "whisper""#));
        // Should update existing parakeet model
        assert!(result.contains(r#"model = "parakeet-tdt-0.6b-v3-int8""#));
        assert!(!result.contains(r#"model = "old-model""#));
    }

    #[test]
    fn test_update_parakeet_preserves_whisper_section() {
        let config = r#"[whisper]
model = "large-v3"
language = "en"
translate = false
"#;
        let result = update_parakeet_in_config(config, "parakeet-tdt-0.6b-v3");

        // Whisper section should be preserved
        assert!(result.contains("[whisper]"));
        assert!(result.contains(r#"model = "large-v3""#));
        assert!(result.contains(r#"language = "en""#));
        // Parakeet section should be added separately
        assert!(result.contains("[parakeet]"));
    }

    #[test]
    fn test_whisper_and_parakeet_models_dont_overlap() {
        // Ensure no model name is valid for both Whisper and Parakeet
        let whisper_names = valid_model_names();
        let parakeet_names = valid_parakeet_model_names();

        for name in &whisper_names {
            assert!(
                !parakeet_names.contains(name),
                "Model '{}' should not be in both Whisper and Parakeet lists",
                name
            );
        }

        for name in &parakeet_names {
            assert!(
                !whisper_names.contains(name),
                "Model '{}' should not be in both Whisper and Parakeet lists",
                name
            );
        }
    }

    // =========================================================================
    // Star Indicator Tests (for model selection menu)
    // =========================================================================

    #[test]
    fn test_star_indicator_whisper_model_selected() {
        use crate::config::TranscriptionEngine;

        // Simulate: engine=Whisper, current model="base.en"
        let is_whisper_engine =
            matches!(TranscriptionEngine::Whisper, TranscriptionEngine::Whisper);
        let current_whisper_model = "base.en";

        // "base.en" should have star
        let is_current = is_whisper_engine && "base.en" == current_whisper_model;
        assert!(
            is_current,
            "base.en should show star when it's the current Whisper model"
        );

        // "small.en" should NOT have star
        let is_current = is_whisper_engine && "small.en" == current_whisper_model;
        assert!(
            !is_current,
            "small.en should not show star when base.en is current"
        );
    }

    #[test]
    fn test_star_indicator_parakeet_model_selected() {
        use crate::config::TranscriptionEngine;

        // Simulate: engine=Parakeet, current model="parakeet-tdt-0.6b-v3"
        let is_parakeet_engine =
            matches!(TranscriptionEngine::Parakeet, TranscriptionEngine::Parakeet);
        let current_parakeet_model: Option<&str> = Some("parakeet-tdt-0.6b-v3");

        // "parakeet-tdt-0.6b-v3" should have star
        let is_current =
            is_parakeet_engine && current_parakeet_model == Some("parakeet-tdt-0.6b-v3");
        assert!(
            is_current,
            "parakeet-tdt-0.6b-v3 should show star when it's the current Parakeet model"
        );

        // "parakeet-tdt-0.6b-v3-int8" should NOT have star
        let is_current =
            is_parakeet_engine && current_parakeet_model == Some("parakeet-tdt-0.6b-v3-int8");
        assert!(
            !is_current,
            "parakeet-tdt-0.6b-v3-int8 should not show star when other model is current"
        );
    }

    #[test]
    fn test_star_indicator_engine_mismatch() {
        use crate::config::TranscriptionEngine;

        // When engine is Parakeet, Whisper models should NOT show star
        let is_whisper_engine =
            matches!(TranscriptionEngine::Parakeet, TranscriptionEngine::Whisper);
        let current_whisper_model = "base.en";

        let is_current = is_whisper_engine && "base.en" == current_whisper_model;
        assert!(
            !is_current,
            "Whisper models should not show star when engine is Parakeet"
        );

        // When engine is Whisper, Parakeet models should NOT show star
        let is_parakeet_engine =
            matches!(TranscriptionEngine::Whisper, TranscriptionEngine::Parakeet);
        let current_parakeet_model: Option<&str> = Some("parakeet-tdt-0.6b-v3");

        let is_current =
            is_parakeet_engine && current_parakeet_model == Some("parakeet-tdt-0.6b-v3");
        assert!(
            !is_current,
            "Parakeet models should not show star when engine is Whisper"
        );
    }

    #[test]
    fn test_star_indicator_no_parakeet_config() {
        use crate::config::TranscriptionEngine;

        // When parakeet config is None (not configured)
        let is_parakeet_engine =
            matches!(TranscriptionEngine::Parakeet, TranscriptionEngine::Parakeet);
        let current_parakeet_model: Option<&str> = None;

        // No model should show star when no parakeet config exists
        let is_current =
            is_parakeet_engine && current_parakeet_model == Some("parakeet-tdt-0.6b-v3");
        assert!(
            !is_current,
            "No star should show when parakeet config is not set"
        );
    }

    // =========================================================================
    // Moonshine Model Tests
    // =========================================================================

    #[test]
    fn test_moonshine_models_list_contains_expected_models() {
        let model_names: Vec<&str> = MOONSHINE_MODELS.iter().map(|m| m.name).collect();
        assert!(model_names.contains(&"base"));
        assert!(model_names.contains(&"tiny"));
    }

    #[test]
    fn test_moonshine_model_info_sizes_are_reasonable() {
        for model in MOONSHINE_MODELS {
            assert!(model.size_mb > 0, "Model {} has invalid size", model.name);
            if model.name.contains("tiny") {
                assert!(model.size_mb <= 150);
            }
            if model.name == "base" {
                assert!(model.size_mb > 150);
            }
        }
    }

    #[test]
    fn test_moonshine_models_have_files() {
        for model in MOONSHINE_MODELS {
            assert!(
                !model.files.is_empty(),
                "Model {} should have file definitions",
                model.name
            );
            // All models should have tokenizer.json
            assert!(
                model
                    .files
                    .iter()
                    .any(|(_, local)| *local == "tokenizer.json"),
                "Model {} should have tokenizer.json",
                model.name
            );
            // All models should have encoder
            assert!(
                model
                    .files
                    .iter()
                    .any(|(_, local)| *local == "encoder_model.onnx"),
                "Model {} should have encoder_model.onnx",
                model.name
            );
        }
    }

    #[test]
    fn test_is_moonshine_model() {
        // Valid Moonshine models
        assert!(is_moonshine_model("base"));
        assert!(is_moonshine_model("tiny"));
        assert!(is_moonshine_model("base-ja"));
        assert!(is_moonshine_model("tiny-ko"));

        // Invalid models
        assert!(!is_moonshine_model("base.en"));
        assert!(!is_moonshine_model("large-v3"));
        assert!(!is_moonshine_model("moonshine"));
        assert!(!is_moonshine_model(""));
    }

    #[test]
    fn test_valid_moonshine_model_names() {
        let names = valid_moonshine_model_names();
        assert!(names.contains(&"base"));
        assert!(names.contains(&"tiny"));
        assert_eq!(names.len(), MOONSHINE_MODELS.len());
    }

    #[test]
    fn test_moonshine_english_models_are_mit() {
        for model in MOONSHINE_MODELS {
            if model.language == "en" {
                assert_eq!(
                    model.license, "MIT",
                    "English model {} should be MIT licensed",
                    model.name
                );
            }
        }
    }

    #[test]
    fn test_moonshine_multilingual_models_are_community() {
        for model in MOONSHINE_MODELS {
            if model.language != "en" {
                assert_eq!(
                    model.license, "Community",
                    "Non-English model {} should be Community licensed",
                    model.name
                );
            }
        }
    }

    #[test]
    fn test_update_moonshine_in_config_basic() {
        let config = r#"engine = "whisper"

[whisper]
model = "base.en"
language = "en"

[output]
mode = "type"
"#;
        let result = update_moonshine_in_config(config, "base");

        // Should update engine to moonshine
        assert!(result.contains(r#"engine = "moonshine""#));
        assert!(!result.contains(r#"engine = "whisper""#));
        // Should add [moonshine] section with model
        assert!(result.contains("[moonshine]"));
        assert!(result.contains(r#"model = "base""#));
        // Should preserve existing sections
        assert!(result.contains("[whisper]"));
        assert!(result.contains("[output]"));
    }

    #[test]
    fn test_update_moonshine_in_config_updates_existing() {
        let config = r#"engine = "whisper"

[whisper]
model = "base.en"

[moonshine]
model = "tiny"
quantized = false

[output]
mode = "type"
"#;
        let result = update_moonshine_in_config(config, "base-ja");

        // Should update engine to moonshine
        assert!(result.contains(r#"engine = "moonshine""#));
        // Should update existing moonshine model
        assert!(result.contains(r#"model = "base-ja""#));
        assert!(!result.contains(r#"model = "tiny""#));
        // Should preserve quantized setting
        assert!(result.contains("quantized = false"));
    }

    #[test]
    fn test_moonshine_and_parakeet_models_dont_overlap() {
        // Moonshine and Parakeet model names should not overlap
        // (Whisper and Moonshine CAN share short names like "tiny" and "base"
        // because they're in different config sections)
        let parakeet_names = valid_parakeet_model_names();
        let moonshine_names = valid_moonshine_model_names();

        for name in &parakeet_names {
            assert!(
                !moonshine_names.contains(name),
                "Model '{}' should not be in both Parakeet and Moonshine lists",
                name
            );
        }
    }

    #[test]
    fn test_moonshine_dir_names_match_convention() {
        for model in MOONSHINE_MODELS {
            assert!(
                model.dir_name.starts_with("moonshine-"),
                "Model dir_name '{}' should start with 'moonshine-'",
                model.dir_name
            );
        }
    }
}

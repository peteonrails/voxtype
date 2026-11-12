//! `voxtype config` subcommand actions.

use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Switch to an installed model and its engine in one write, without downloading or restarting
    #[command(long_about = "\
        Switch to an installed model and its engine in one write, without \
        downloading or restarting\n\n\
        Sets `engine` and `<ENGINE>.model` together, so a frontend can move \
        from one engine's model to another's in a single validated save. \
        Works for every engine `voxtype config set engine` accepts: whisper, \
        parakeet, moonshine, sensevoice, paraformer, dolphin, omnilingual, \
        cohere, gguf and openvino. The model must already be downloaded \
        (`voxtype info models` shows what is installed), and the engine must \
        be compiled into this binary.\n\n\
        Nothing is written when the selection is refused: the config file is \
        left byte-identical. Restart the daemon separately to apply it.\n\n\
        Exit codes: 0 on success, 2 for an unknown engine, an engine this \
        binary lacks, or a model that is missing or incompatible, 1 for \
        filesystem errors.\n\n\
        Examples:\n  \
        voxtype config select-model parakeet parakeet-ultra-Q8_0.gguf\n  \
        voxtype config select-model omnilingual omnilingual-300m\n  \
        voxtype config select-model whisper large-v3-turbo\n  \
        voxtype config select-model gguf Qwen3-ASR-0.6B-Q8_0.gguf --language en")]
    SelectModel {
        /// Engine to switch to, e.g. parakeet, whisper, moonshine, gguf
        engine: String,
        /// Installed model of that engine, as `voxtype info models` names it, or a model path
        model: String,
        /// Explicit GGUF language hint (engine gguf only); omitted preserves a compatible saved hint
        #[arg(long)]
        language: Option<String>,
    },

    /// Check the active engine/model/backend and installed worker without loading audio
    ValidateSelection {
        /// Refuse while a running daemon is busy or its idle state cannot be verified
        #[arg(long)]
        require_idle: bool,
    },

    /// Set a single configuration value in the on-disk config file
    ///
    /// Comments and unrelated fields are preserved. Most keys need a daemon
    /// restart to take effect; the command says so when they do.
    // Deliberately no engine list here: `ENGINE_NAMES_CSV` enumerates every
    // TranscriptionEngine variant, including ones `config set engine` does
    // not accept. `voxtype info engines` reports what this binary can
    // actually switch to.
    #[command(long_about = "\
        Set a single configuration value in the on-disk config file\n\n\
        KEY is a dotted TOML path from the allowlist printed by \
        `voxtype config schema`; VALUE is type-checked and range-checked \
        against that key before anything is written. Comments and unrelated \
        fields are preserved.\n\n\
        Word replacements are addressed by their spoken form: \
        `text.replacements.<from>`.\n\n\
        Exit codes: 0 on success, 2 for an unknown key, an invalid value, or \
        a key belonging to an engine this binary wasn't built with, 1 for \
        filesystem and validation failures.\n\n\
        Examples:\n  \
        voxtype config set engine whisper\n  \
        voxtype config set hotkey.key F13\n  \
        voxtype config set audio.feedback.volume 0.6\n  \
        voxtype config set text.replacements.btw 'by the way'\n\n\
        See `voxtype info engines` for the engines this binary supports.")]
    Set {
        /// Dotted config key, e.g. hotkey.mode or audio.feedback.volume
        #[arg(value_name = "KEY")]
        key: String,

        /// New value. Booleans accept true/false, 1/0, yes/no, on/off.
        #[arg(value_name = "VALUE")]
        value: String,
    },

    /// Remove a configuration value, restoring its built-in default
    #[command(long_about = "\
        Remove a configuration value, restoring its built-in default\n\n\
        Removing a key that isn't present succeeds. Use this rather than \
        setting an empty string — an empty value is rejected, because for \
        most keys it isn't the same thing as \"unset\".\n\n\
        Examples:\n  \
        voxtype config unset whisper.initial_prompt\n  \
        voxtype config unset text.replacements.btw")]
    Unset {
        /// Dotted config key to remove
        #[arg(value_name = "KEY")]
        key: String,
    },

    /// Print resolved configuration values
    #[command(long_about = "\
        Print resolved configuration values\n\n\
        With a KEY, prints that key's effective value (defaults applied). \
        Without one, prints every allowlisted key. Add --json for machine-\
        readable output; with a KEY that also reports the literal value \
        present in the config file, which is null when the key is absent and \
        the default is in play.\n\n\
        Examples:\n  \
        voxtype config get hotkey.mode\n  \
        voxtype config get audio.device --json\n  \
        voxtype config get --json")]
    Get {
        /// Dotted config key. Omit to print every allowlisted key.
        #[arg(value_name = "KEY")]
        key: Option<String>,

        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },

    /// Describe every settable configuration key
    ///
    /// Emits the key allowlist with types, valid values, current values, and
    /// which keys this binary can actually honor. Intended for building
    /// settings UIs against.
    Schema {
        /// Emit machine-readable JSON instead of human-readable text
        #[arg(long)]
        json: bool,
    },
}

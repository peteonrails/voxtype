# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_voxtype_global_optspecs
    string join \n c/config= v/verbose q/quiet model= engine= language= translate threads= gpu-isolation gpu-device= on-demand-loading secondary-model= eager-processing no-whisper-context-optimization initial-prompt= flash-attention whisper-mode= remote-endpoint= remote-model= remote-api-key= soniox-api-key= hotkey= toggle no-hotkey cancel-key= model-modifier= audio-device= max-duration= audio-feedback no-audio-feedback pause-media duck-media duck-media-volume= duck-media-fade-ms= clipboard paste restore-clipboard restore-clipboard-delay-ms= driver= auto-submit no-auto-submit fallback-to-clipboard no-fallback-to-clipboard paste-keys= file-path= file-mode= pre-type-delay= wtype-delay= wtype-shift-prefix type-delay= dotool-xkb-layout= dotool-xkb-variant= eitype-xkb-layout= eitype-xkb-variant= pre-output-command= post-output-command= pre-recording-command= wait-for-modifier-release no-wait-for-modifier-release modifier-release-timeout-ms= spoken-punctuation shift-enter-newlines no-shift-enter-newlines smart-auto-submit no-smart-auto-submit filter-fillers no-filter-fillers append-text= vad vad-threshold= vad-backend= vad-min-speech-ms= h/help V/version
end

function __fish_voxtype_needs_command
    # Figure out if the current invocation already has a command.
    set -l cmd (commandline -opc)
    set -e cmd[1]
    argparse -s (__fish_voxtype_global_optspecs) -- $cmd 2>/dev/null
    or return
    if set -q argv[1]
        # Also print the command, so this can be used to figure out what it is.
        echo $argv[1]
        return 1
    end
    return 0
end

function __fish_voxtype_using_subcommand
    set -l cmd (__fish_voxtype_needs_command)
    test -z "$cmd"
    and return 1
    contains -- $cmd[1] $argv
end

complete -c voxtype -n "__fish_voxtype_needs_command" -s c -l config -d 'Path to config file' -r -F
complete -c voxtype -n "__fish_voxtype_needs_command" -l model -d 'Override transcription model' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l engine -d 'Override transcription engine' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l language -d 'Language for transcription (e.g., en, fr, auto, or comma-separated: en,fr,de)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l threads -d 'Number of CPU threads for inference' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l gpu-device -d 'GPU device index for multi-GPU systems (e.g., 1 for discrete GPU)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l secondary-model -d 'Secondary model for difficult audio (used with --model-modifier)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l initial-prompt -d 'Initial prompt to provide context for transcription' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l whisper-mode -d 'Whisper execution mode: local, remote, or cli' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l remote-endpoint -d 'Remote server endpoint URL (for remote whisper mode)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l remote-model -d 'Model name to send to remote server' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l remote-api-key -d 'API key for remote server (or use VOXTYPE_WHISPER_API_KEY env var)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l soniox-api-key -d 'API key for Soniox (or use SONIOX_API_KEY env var)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l hotkey -d 'Override hotkey (e.g., SCROLLLOCK, PAUSE, F13, MEDIA, WEV_234, EVTEST_226)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l cancel-key -d 'Cancel key for aborting recording or transcription (e.g., ESC, BACKSPACE, F12)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l model-modifier -d 'Modifier key for secondary model selection (e.g., LEFTSHIFT)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l audio-device -d 'Audio input device name (or "default" for system default)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l max-duration -d 'Maximum recording duration in seconds (safety limit)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l duck-media-volume -d 'Percent of its current amplitude ducked media keeps (50 = half, -6 dB)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l duck-media-fade-ms -d 'Milliseconds to fade ducked media down and back up (0 = instant)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l restore-clipboard-delay-ms -d 'Delay in milliseconds after paste before restoring clipboard (default: 200)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l driver -d 'Output driver order (comma-separated)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l paste-keys -d 'Keystroke for paste mode (e.g., ctrl+v, shift+insert, ctrl+shift+v)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l file-path -d 'File path for file output mode' -r -F
complete -c voxtype -n "__fish_voxtype_needs_command" -l file-mode -d 'File write mode: overwrite or append' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l pre-type-delay -d 'Delay before typing starts (ms), helps prevent first character drop' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l wtype-delay -d 'DEPRECATED: Use --pre-type-delay instead' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l type-delay -d 'Delay between typed characters in milliseconds (0 = fastest)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l dotool-xkb-layout -d 'Keyboard layout for dotool (e.g., de, fr)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l dotool-xkb-variant -d 'Keyboard layout variant for dotool (e.g., nodeadkeys)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l eitype-xkb-layout -d 'Keyboard layout for eitype (e.g., de, ru, us). Passed as `-l <LAYOUT>`. Overrides any layout derived from the transcribed language' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l eitype-xkb-variant -d 'Keyboard layout variant for eitype (e.g., dvorak, colemak)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l pre-output-command -d 'Command to run before typing output (e.g., compositor submap switch)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l post-output-command -d 'Command to run after typing output (e.g., reset compositor submap)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l pre-recording-command -d 'Command to run when recording starts (e.g., switch to compositor submap)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l modifier-release-timeout-ms -d 'Maximum milliseconds to wait for modifier release before falling back to clipboard' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l append-text -d 'Text to append after each transcription (e.g., " " for trailing space)' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l vad-threshold -d 'VAD speech detection threshold (0.0-1.0, default: 0.5). Lower = more sensitive, Higher = less sensitive' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l vad-backend -d 'VAD backend: auto, energy, whisper' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -l vad-min-speech-ms -d 'Minimum speech duration in milliseconds for VAD' -r
complete -c voxtype -n "__fish_voxtype_needs_command" -s v -l verbose -d 'Increase verbosity (-v = debug, -vv = trace)'
complete -c voxtype -n "__fish_voxtype_needs_command" -s q -l quiet -d 'Quiet mode (errors only)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l translate -d 'Translate non-English speech to English'
complete -c voxtype -n "__fish_voxtype_needs_command" -l gpu-isolation -d 'Run transcription in a subprocess to release GPU memory after each recording'
complete -c voxtype -n "__fish_voxtype_needs_command" -l on-demand-loading -d 'Load model on-demand when recording starts instead of keeping it loaded'
complete -c voxtype -n "__fish_voxtype_needs_command" -l eager-processing -d 'Enable eager input processing (transcribe chunks while recording continues)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-whisper-context-optimization -d 'Disable context window optimization for short recordings'
complete -c voxtype -n "__fish_voxtype_needs_command" -l flash-attention -d 'Enable flash attention for reduced GPU memory usage and faster inference'
complete -c voxtype -n "__fish_voxtype_needs_command" -l toggle -d 'Use toggle mode (press to start/stop) instead of push-to-talk (hold to record)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-hotkey -d 'Disable built-in hotkey detection (use compositor keybindings instead)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l audio-feedback -d 'Enable audio feedback sounds (beeps when recording starts/stops)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-audio-feedback -d 'Disable audio feedback sounds'
complete -c voxtype -n "__fish_voxtype_needs_command" -l pause-media -d 'Pause MPRIS media players during recording'
complete -c voxtype -n "__fish_voxtype_needs_command" -l duck-media -d 'Lower active media volume during recording instead of pausing it'
complete -c voxtype -n "__fish_voxtype_needs_command" -l clipboard -d 'Force clipboard mode (don\'t try to type)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l paste -d 'Force paste mode (clipboard + Ctrl+V)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l restore-clipboard -d 'Restore clipboard after paste mode'
complete -c voxtype -n "__fish_voxtype_needs_command" -l auto-submit -d 'Auto-submit (press Enter) after outputting transcribed text'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-auto-submit -d 'Disable auto-submit (overrides config auto_submit = true)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l fallback-to-clipboard -d 'Fall back to clipboard if typing fails'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-fallback-to-clipboard -d 'Disable clipboard fallback'
complete -c voxtype -n "__fish_voxtype_needs_command" -l wtype-shift-prefix -d 'Prefix wtype output with a Shift key press/release'
complete -c voxtype -n "__fish_voxtype_needs_command" -l wait-for-modifier-release -d 'Wait for modifier keys (Ctrl/Alt/Shift/Super) to be released before typing'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-wait-for-modifier-release -d 'Disable waiting for modifier release (overrides config)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l spoken-punctuation -d 'Enable spoken punctuation conversion (e.g., say "period" to get ".")'
complete -c voxtype -n "__fish_voxtype_needs_command" -l shift-enter-newlines -d 'Convert newlines to Shift+Enter instead of regular Enter'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-shift-enter-newlines -d 'Disable Shift+Enter newlines (overrides config)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l smart-auto-submit -d 'Enable smart auto-submit (say "submit" to press Enter)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-smart-auto-submit -d 'Disable smart auto-submit (overrides config)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l filter-fillers -d 'Filter common filler words ("uh", "um", "er", ...) from transcribed text'
complete -c voxtype -n "__fish_voxtype_needs_command" -l no-filter-fillers -d 'Disable filler-word filtering (overrides config)'
complete -c voxtype -n "__fish_voxtype_needs_command" -l vad -d 'Enable Voice Activity Detection (filter silence before transcription)'
complete -c voxtype -n "__fish_voxtype_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_needs_command" -s V -l version -d 'Print version'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "daemon" -d 'Run as daemon (default if no command specified)'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "transcribe" -d 'Transcribe an audio file (WAV, 16kHz, mono)'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "transcribe-worker" -d 'Internal: Worker process for GPU-isolated transcription Reads audio from stdin, writes transcription result to stdout'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "setup" -d 'Setup and installation utilities'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "config" -d 'Show or modify configuration'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "info" -d 'Inspect runtime/install information'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "configure" -d 'Open the interactive configuration TUI'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "status" -d 'Show daemon status (for Waybar/polybar integration)'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "record" -d 'Control recording from external sources (compositor keybindings, scripts)'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "meeting" -d 'Meeting transcription mode'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "check-update" -d 'Check for updates'
complete -c voxtype -n "__fish_voxtype_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand daemon" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand transcribe" -l engine -d 'Override transcription engine' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand transcribe" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand transcribe-worker" -l model -d 'Model name or path (passed from parent process)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand transcribe-worker" -l language -d 'Language code (passed from parent process)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand transcribe-worker" -l threads -d 'Number of threads for inference (passed from parent process)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand transcribe-worker" -l translate -d 'Enable translation to English (passed from parent process)'
complete -c voxtype -n "__fish_voxtype_using_subcommand transcribe-worker" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -l model -d 'Specify which model to download (use with --download). Whisper: tiny, base, small, medium, large-v3, large-v3-turbo (and .en variants). Parakeet: parakeet-tdt-0.6b-v3, parakeet-tdt-0.6b-v3-int8. Other engines (Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual, Cohere, OpenVINO) take the directory-form names shown by `voxtype info models`, e.g. cohere-transcribe-q4f16' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -l progress-format -d 'How download progress is reported: human (curl\'s progress bar) or json (one NDJSON event per update on stdout, for a GUI to render)' -r -f -a "human\t''
json\t''"
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -l download -d 'Download model if missing (shorthand for basic setup)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -l quiet -d 'Suppress all output (for scripting/automation)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -l no-post-install -d 'Suppress only "Next steps" instructions'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -l activate -d 'Also switch the config to use the model, the way the interactive picker does: sets `engine` and `<engine>.model`'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "check" -d 'Check system configuration and dependencies'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "systemd" -d 'Install voxtype as a systemd user service (Linux)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "waybar" -d 'Show Waybar configuration snippets'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "dms" -d 'DankMaterialShell (DMS) integration'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "model" -d 'Interactive model selection and download'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "gpu" -d 'Manage GPU acceleration (Vulkan for Whisper, CUDA/MIGraphX for Parakeet)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "npu" -d 'Manage Intel NPU acceleration through OpenVINO'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "variant" -d 'Switch the active binary variant (used by `voxtype configure` via pkexec)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "onnx" -d 'Switch between Whisper and ONNX transcription engines'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "parakeet" -d 'Hidden alias for \'onnx\' (backwards compatibility)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "compositor" -d 'Compositor integration (fixes modifier key interference)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "vad" -d 'Download the Silero VAD model for speech detection'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "quickshell" -d 'Install the Quickshell QML tree for the voxtype-osd-quickshell launcher'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and not __fish_seen_subcommand_from check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from check" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from systemd" -l uninstall -d 'Uninstall the service instead of installing'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from systemd" -l status -d 'Show service status'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from systemd" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from waybar" -l json -d 'Output only the JSON config (for scripting)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from waybar" -l css -d 'Output only the CSS config (for scripting)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from waybar" -l install -d 'Install waybar integration (inject config and CSS)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from waybar" -l uninstall -d 'Uninstall waybar integration (remove config and CSS)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from waybar" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from dms" -l install -d 'Install DMS plugin (create widget directory and QML file)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from dms" -l uninstall -d 'Uninstall DMS plugin (remove widget directory)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from dms" -l qml -d 'Output only the QML content (for scripting)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from dms" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from model" -l set -d 'Set a specific model as default (must already be downloaded)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from model" -l list -d 'List installed models instead of interactive selection'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from model" -l restart -d 'Restart the daemon after changing model (use with --set)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from model" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from gpu" -l enable -d 'Enable GPU acceleration (auto-detects best backend)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from gpu" -l disable -d 'Disable GPU acceleration (switch back to CPU)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from gpu" -l status -d 'Show current backend status'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from gpu" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from npu" -l enable -d 'Enable NPU acceleration and configure OpenVINO'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from npu" -l disable -d 'Disable NPU acceleration and revert to Whisper'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from npu" -l status -d 'Show NPU hardware and configuration status'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from npu" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from variant" -l to -d 'Variant binary name (e.g., voxtype-avx512, voxtype-onnx-cuda)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from variant" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from onnx" -l enable -d 'Enable ONNX engine (switch to ONNX binary)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from onnx" -l disable -d 'Disable ONNX engine (switch back to Whisper binary)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from onnx" -l status -d 'Show current ONNX backend status'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from onnx" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from parakeet" -l enable
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from parakeet" -l disable
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from parakeet" -l status
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from parakeet" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from compositor" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from compositor" -f -a "hyprland" -d 'Hyprland compositor configuration'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from compositor" -f -a "sway" -d 'Sway compositor configuration'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from compositor" -f -a "river" -d 'River compositor configuration'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from compositor" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from vad" -l status -d 'Show VAD model status'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from vad" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -l target -d 'Override the install target directory' -r -F
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -l source -d 'Override the QML source directory (otherwise auto-detected)' -r -F
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -l bridge -d 'Override the source path of the voxtype-audio-bridge binary' -r -F
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -l bridge-target -d 'Override the symlink location for voxtype-audio-bridge' -r -F
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -l force -d 'Overwrite an existing install at the target'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -l print-bindings -d 'Skip the file copy; only print the compositor binding examples'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -l skip-bridge -d 'Skip installing the voxtype-audio-bridge symlink'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from quickshell" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "check" -d 'Check system configuration and dependencies'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "systemd" -d 'Install voxtype as a systemd user service (Linux)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "waybar" -d 'Show Waybar configuration snippets'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "dms" -d 'DankMaterialShell (DMS) integration'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "model" -d 'Interactive model selection and download'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "gpu" -d 'Manage GPU acceleration (Vulkan for Whisper, CUDA/MIGraphX for Parakeet)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "npu" -d 'Manage Intel NPU acceleration through OpenVINO'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "variant" -d 'Switch the active binary variant (used by `voxtype configure` via pkexec)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "onnx" -d 'Switch between Whisper and ONNX transcription engines'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "parakeet" -d 'Hidden alias for \'onnx\' (backwards compatibility)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "compositor" -d 'Compositor integration (fixes modifier key interference)'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "vad" -d 'Download the Silero VAD model for speech detection'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "quickshell" -d 'Install the Quickshell QML tree for the voxtype-osd-quickshell launcher'
complete -c voxtype -n "__fish_voxtype_using_subcommand setup; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and not __fish_seen_subcommand_from set unset get schema help" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and not __fish_seen_subcommand_from set unset get schema help" -f -a "set" -d 'Set a single configuration value in the on-disk config file'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and not __fish_seen_subcommand_from set unset get schema help" -f -a "unset" -d 'Remove a configuration value, restoring its built-in default'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and not __fish_seen_subcommand_from set unset get schema help" -f -a "get" -d 'Print resolved configuration values'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and not __fish_seen_subcommand_from set unset get schema help" -f -a "schema" -d 'Describe every settable configuration key'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and not __fish_seen_subcommand_from set unset get schema help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from set" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from unset" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from get" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from schema" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from schema" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "set" -d 'Set a single configuration value in the on-disk config file'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "unset" -d 'Remove a configuration value, restoring its built-in default'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "get" -d 'Print resolved configuration values'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "schema" -d 'Describe every settable configuration key'
complete -c voxtype -n "__fish_voxtype_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -f -a "variants" -d 'Show installed binary variants and which one is active'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -f -a "devices" -d 'List audio capture devices'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -f -a "models" -d 'List downloadable models per engine and which are installed'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -f -a "accel" -d 'Report whether the running daemon is GPU-accelerated'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -f -a "engines" -d 'List transcription engines and which are compiled into this binary'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -f -a "styles" -d 'List installed OSD styles (Quickshell frontend)'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and not __fish_seen_subcommand_from variants devices models accel engines styles help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from variants" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from variants" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from devices" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from devices" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from models" -l engine -d 'Restrict output to one engine' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from models" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from models" -l verify -d 'Also hash every file of every installed model against the manifest recorded at download time. Thorough and slow: it reads every byte of every model, which is minutes for a full models directory'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from models" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from accel" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from accel" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from engines" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from engines" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from styles" -l json -d 'Emit machine-readable JSON instead of human-readable text'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from styles" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from help" -f -a "variants" -d 'Show installed binary variants and which one is active'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from help" -f -a "devices" -d 'List audio capture devices'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from help" -f -a "models" -d 'List downloadable models per engine and which are installed'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from help" -f -a "accel" -d 'Report whether the running daemon is GPU-accelerated'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from help" -f -a "engines" -d 'List transcription engines and which are compiled into this binary'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from help" -f -a "styles" -d 'List installed OSD styles (Quickshell frontend)'
complete -c voxtype -n "__fish_voxtype_using_subcommand info; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand configure" -l force-package-mode -d 'Render as if installed from a package (for testing source builds)'
complete -c voxtype -n "__fish_voxtype_using_subcommand configure" -l probe-audio-devices -d 'Print detected audio input devices (one per line) and exit. Used internally by the TUI to probe devices in a subprocess, so an ALSA hang or crash during probing can\'t take the TUI with it'
complete -c voxtype -n "__fish_voxtype_using_subcommand configure" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand status" -l format -d 'Output format: "text" (default) or "json" (for Waybar)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand status" -l icon-theme -d 'Icon theme for JSON output (emoji, nerd-font, material, phosphor, codicons, omarchy, minimal, dots, arrows, text, or path to custom theme)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand status" -l follow -d 'Continuously output status changes as JSON (for Waybar exec)'
complete -c voxtype -n "__fish_voxtype_using_subcommand status" -l extended -d 'Include extended info in JSON (model, device, backend)'
complete -c voxtype -n "__fish_voxtype_using_subcommand status" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and not __fish_seen_subcommand_from start stop toggle cancel help" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and not __fish_seen_subcommand_from start stop toggle cancel help" -f -a "start" -d 'Start recording (send SIGUSR1 to daemon)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and not __fish_seen_subcommand_from start stop toggle cancel help" -f -a "stop" -d 'Stop recording and transcribe (send SIGUSR2 to daemon)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and not __fish_seen_subcommand_from start stop toggle cancel help" -f -a "toggle" -d 'Toggle recording state'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and not __fish_seen_subcommand_from start stop toggle cancel help" -f -a "cancel" -d 'Cancel current recording or transcription (discard without output)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and not __fish_seen_subcommand_from start stop toggle cancel help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l file -d 'Write transcription to a file Use --file alone to use file_path from config, or --file=path.txt for explicit path' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l model -d 'Use a specific model for this transcription (e.g., large-v3-turbo)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l profile -d 'Use a named profile for post-processing (e.g., --profile slack) Profiles are defined in config.toml under [profiles.name]' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l type -d 'Override output mode to simulate keyboard typing'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l clipboard -d 'Override output mode to clipboard only'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l paste -d 'Override output mode to paste (clipboard + Ctrl+V)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l auto-submit -d 'Auto-submit (press Enter) after this transcription'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l no-auto-submit -d 'Disable auto-submit for this transcription (overrides config)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l shift-enter-newlines -d 'Use Shift+Enter for newlines in this transcription'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l no-shift-enter-newlines -d 'Disable Shift+Enter newlines for this transcription (overrides config)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l no-osd -d 'Suppress the on-screen display for this recording only'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l smart-auto-submit -d 'Enable smart auto-submit for this recording (say "submit" to press Enter)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -l no-smart-auto-submit -d 'Disable smart auto-submit for this recording'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from start" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -l timeout -d 'With --wait, give up after this many seconds' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -l wait-file -d 'With --wait, the transcript path to wait on' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -l type -d 'Override output mode to simulate keyboard typing'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -l clipboard -d 'Override output mode to clipboard only'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -l paste -d 'Override output mode to paste (clipboard + Ctrl+V)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -l wait -d 'Block until the transcription is final instead of returning as soon as the daemon has been signalled'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -l json -d 'With --wait, print one JSON object describing the outcome'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from stop" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l file -d 'Write transcription to a file Use --file alone to use file_path from config, or --file=path.txt for explicit path' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l model -d 'Use a specific model for this transcription (e.g., large-v3-turbo)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l profile -d 'Use a named profile for post-processing (e.g., --profile slack) Profiles are defined in config.toml under [profiles.name]' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l type -d 'Override output mode to simulate keyboard typing'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l clipboard -d 'Override output mode to clipboard only'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l paste -d 'Override output mode to paste (clipboard + Ctrl+V)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l auto-submit -d 'Auto-submit (press Enter) after this transcription'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l no-auto-submit -d 'Disable auto-submit for this transcription (overrides config)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l shift-enter-newlines -d 'Use Shift+Enter for newlines in this transcription'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l no-shift-enter-newlines -d 'Disable Shift+Enter newlines for this transcription (overrides config)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l no-osd -d 'Suppress the on-screen display for this recording only'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l smart-auto-submit -d 'Enable smart auto-submit for this recording (say "submit" to press Enter)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -l no-smart-auto-submit -d 'Disable smart auto-submit for this recording (overrides config)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from toggle" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from cancel" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from help" -f -a "start" -d 'Start recording (send SIGUSR1 to daemon)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from help" -f -a "stop" -d 'Stop recording and transcribe (send SIGUSR2 to daemon)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from help" -f -a "toggle" -d 'Toggle recording state'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from help" -f -a "cancel" -d 'Cancel current recording or transcription (discard without output)'
complete -c voxtype -n "__fish_voxtype_using_subcommand record; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "start" -d 'Start a new meeting transcription'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "stop" -d 'Stop the current meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "pause" -d 'Pause the current meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "resume" -d 'Resume a paused meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "status" -d 'Show meeting status'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "list" -d 'List past meetings'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "export" -d 'Export a meeting transcript'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "show" -d 'Show meeting details'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "delete" -d 'Delete a meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "label" -d 'Label a speaker in a meeting transcript'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "summarize" -d 'Generate an AI summary of a meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and not __fish_seen_subcommand_from start stop pause resume status list export show delete label summarize help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from start" -s t -l title -d 'Meeting title (optional)' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from start" -l diarization -d 'Diarization backend override for this meeting only' -r -f -a "simple\t''
ml\t''"
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from start" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from stop" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from pause" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from resume" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from list" -s l -l limit -d 'Maximum number of meetings to show' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from export" -s f -l format -d 'Output format: text, markdown, json' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from export" -s o -l output -d 'Output file path (default: stdout)' -r -F
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from export" -l timestamps -d 'Include timestamps in output'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from export" -l speakers -d 'Include speaker labels in output'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from export" -l metadata -d 'Include metadata header in output'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from export" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from show" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from delete" -s f -l force -d 'Skip confirmation prompt'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from label" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from summarize" -s f -l format -d 'Output format: text, json, or markdown' -r
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from summarize" -s o -l output -d 'Output file path (default: stdout)' -r -F
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from summarize" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "start" -d 'Start a new meeting transcription'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "stop" -d 'Stop the current meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "pause" -d 'Pause the current meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "resume" -d 'Resume a paused meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "status" -d 'Show meeting status'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "list" -d 'List past meetings'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "export" -d 'Export a meeting transcript'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "show" -d 'Show meeting details'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "delete" -d 'Delete a meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "label" -d 'Label a speaker in a meeting transcript'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "summarize" -d 'Generate an AI summary of a meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand meeting; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand check-update" -s h -l help -d 'Print help'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "daemon" -d 'Run as daemon (default if no command specified)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "transcribe" -d 'Transcribe an audio file (WAV, 16kHz, mono)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "transcribe-worker" -d 'Internal: Worker process for GPU-isolated transcription Reads audio from stdin, writes transcription result to stdout'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "setup" -d 'Setup and installation utilities'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "config" -d 'Show or modify configuration'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "info" -d 'Inspect runtime/install information'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "configure" -d 'Open the interactive configuration TUI'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "status" -d 'Show daemon status (for Waybar/polybar integration)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "record" -d 'Control recording from external sources (compositor keybindings, scripts)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "meeting" -d 'Meeting transcription mode'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "check-update" -d 'Check for updates'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and not __fish_seen_subcommand_from daemon transcribe transcribe-worker setup config info configure status record meeting check-update help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "check" -d 'Check system configuration and dependencies'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "systemd" -d 'Install voxtype as a systemd user service (Linux)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "waybar" -d 'Show Waybar configuration snippets'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "dms" -d 'DankMaterialShell (DMS) integration'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "model" -d 'Interactive model selection and download'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "gpu" -d 'Manage GPU acceleration (Vulkan for Whisper, CUDA/MIGraphX for Parakeet)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "npu" -d 'Manage Intel NPU acceleration through OpenVINO'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "variant" -d 'Switch the active binary variant (used by `voxtype configure` via pkexec)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "onnx" -d 'Switch between Whisper and ONNX transcription engines'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "parakeet" -d 'Hidden alias for \'onnx\' (backwards compatibility)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "compositor" -d 'Compositor integration (fixes modifier key interference)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "vad" -d 'Download the Silero VAD model for speech detection'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from setup" -f -a "quickshell" -d 'Install the Quickshell QML tree for the voxtype-osd-quickshell launcher'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "set" -d 'Set a single configuration value in the on-disk config file'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "unset" -d 'Remove a configuration value, restoring its built-in default'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "get" -d 'Print resolved configuration values'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "schema" -d 'Describe every settable configuration key'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from info" -f -a "variants" -d 'Show installed binary variants and which one is active'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from info" -f -a "devices" -d 'List audio capture devices'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from info" -f -a "models" -d 'List downloadable models per engine and which are installed'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from info" -f -a "accel" -d 'Report whether the running daemon is GPU-accelerated'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from info" -f -a "engines" -d 'List transcription engines and which are compiled into this binary'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from info" -f -a "styles" -d 'List installed OSD styles (Quickshell frontend)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from record" -f -a "start" -d 'Start recording (send SIGUSR1 to daemon)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from record" -f -a "stop" -d 'Stop recording and transcribe (send SIGUSR2 to daemon)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from record" -f -a "toggle" -d 'Toggle recording state'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from record" -f -a "cancel" -d 'Cancel current recording or transcription (discard without output)'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "start" -d 'Start a new meeting transcription'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "stop" -d 'Stop the current meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "pause" -d 'Pause the current meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "resume" -d 'Resume a paused meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "status" -d 'Show meeting status'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "list" -d 'List past meetings'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "export" -d 'Export a meeting transcript'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "show" -d 'Show meeting details'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "delete" -d 'Delete a meeting'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "label" -d 'Label a speaker in a meeting transcript'
complete -c voxtype -n "__fish_voxtype_using_subcommand help; and __fish_seen_subcommand_from meeting" -f -a "summarize" -d 'Generate an AI summary of a meeting'

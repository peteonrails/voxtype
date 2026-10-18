#compdef voxtype

autoload -U is-at-least

_voxtype() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
'-c+[Path to config file]:FILE:_files' \
'--config=[Path to config file]:FILE:_files' \
'--model=[Override transcription model]:MODEL:_default' \
'--engine=[Override transcription engine]:ENGINE:_default' \
'--language=[Language for transcription (e.g., en, fr, auto, or comma-separated\: en,fr,de)]:LANG:_default' \
'--threads=[Number of CPU threads for inference]:N:_default' \
'--gpu-device=[GPU device index for multi-GPU systems (e.g., 1 for discrete GPU)]:INDEX:_default' \
'--secondary-model=[Secondary model for difficult audio (used with --model-modifier)]:MODEL:_default' \
'--initial-prompt=[Initial prompt to provide context for transcription]:PROMPT:_default' \
'--whisper-mode=[Whisper execution mode\: local, remote, or cli]:MODE:_default' \
'--remote-endpoint=[Remote server endpoint URL (for remote whisper mode)]:URL:_default' \
'--remote-model=[Model name to send to remote server]:MODEL:_default' \
'--remote-api-key=[API key for remote server (or use VOXTYPE_WHISPER_API_KEY env var)]:KEY:_default' \
'--soniox-api-key=[API key for Soniox (or use SONIOX_API_KEY env var)]:KEY:_default' \
'--hotkey=[Override hotkey (e.g., SCROLLLOCK, PAUSE, F13, MEDIA, WEV_234, EVTEST_226)]:KEY:_default' \
'--cancel-key=[Cancel key for aborting recording or transcription (e.g., ESC, BACKSPACE, F12)]:KEY:_default' \
'--model-modifier=[Modifier key for secondary model selection (e.g., LEFTSHIFT)]:KEY:_default' \
'--audio-device=[Audio input device name (or "default" for system default)]:DEVICE:_default' \
'--max-duration=[Maximum recording duration in seconds (safety limit)]:SECS:_default' \
'--duck-media-volume=[Percent of its current amplitude ducked media keeps (50 = half, -6 dB)]:PERCENT:_default' \
'--duck-media-fade-ms=[Milliseconds to fade ducked media down and back up (0 = instant)]:MS:_default' \
'--restore-clipboard-delay-ms=[Delay in milliseconds after paste before restoring clipboard (default\: 200)]:MS:_default' \
'--driver=[Output driver order (comma-separated)]:DRIVERS:_default' \
'--paste-keys=[Keystroke for paste mode (e.g., ctrl+v, shift+insert, ctrl+shift+v)]:KEYS:_default' \
'--file-path=[File path for file output mode]:PATH:_files' \
'--file-mode=[File write mode\: overwrite or append]:MODE:_default' \
'--pre-type-delay=[Delay before typing starts (ms), helps prevent first character drop]:MS:_default' \
'--wtype-delay=[DEPRECATED\: Use --pre-type-delay instead]:MS:_default' \
'--type-delay=[Delay between typed characters in milliseconds (0 = fastest)]:MS:_default' \
'--dotool-xkb-layout=[Keyboard layout for dotool (e.g., de, fr)]:LAYOUT:_default' \
'--dotool-xkb-variant=[Keyboard layout variant for dotool (e.g., nodeadkeys)]:VARIANT:_default' \
'--eitype-xkb-layout=[Keyboard layout for eitype (e.g., de, ru, us). Passed as \`-l <LAYOUT>\`. Overrides any layout derived from the transcribed language]:LAYOUT:_default' \
'--eitype-xkb-variant=[Keyboard layout variant for eitype (e.g., dvorak, colemak)]:VARIANT:_default' \
'--pre-output-command=[Command to run before typing output (e.g., compositor submap switch)]:CMD:_default' \
'--post-output-command=[Command to run after typing output (e.g., reset compositor submap)]:CMD:_default' \
'--pre-recording-command=[Command to run when recording starts (e.g., switch to compositor submap)]:CMD:_default' \
'--modifier-release-timeout-ms=[Maximum milliseconds to wait for modifier release before falling back to clipboard]:MS:_default' \
'--append-text=[Text to append after each transcription (e.g., " " for trailing space)]:TEXT:_default' \
'--vad-threshold=[VAD speech detection threshold (0.0-1.0, default\: 0.5). Lower = more sensitive, Higher = less sensitive]:THRESHOLD:_default' \
'--vad-backend=[VAD backend\: auto, energy, whisper]:BACKEND:_default' \
'--vad-min-speech-ms=[Minimum speech duration in milliseconds for VAD]:MS:_default' \
'*-v[Increase verbosity (-v = debug, -vv = trace)]' \
'*--verbose[Increase verbosity (-v = debug, -vv = trace)]' \
'-q[Quiet mode (errors only)]' \
'--quiet[Quiet mode (errors only)]' \
'--translate[Translate non-English speech to English]' \
'--gpu-isolation[Run transcription in a subprocess to release GPU memory after each recording]' \
'--on-demand-loading[Load model on-demand when recording starts instead of keeping it loaded]' \
'--eager-processing[Enable eager input processing (transcribe chunks while recording continues)]' \
'--no-whisper-context-optimization[Disable context window optimization for short recordings]' \
'--flash-attention[Enable flash attention for reduced GPU memory usage and faster inference]' \
'--toggle[Use toggle mode (press to start/stop) instead of push-to-talk (hold to record)]' \
'--no-hotkey[Disable built-in hotkey detection (use compositor keybindings instead)]' \
'--audio-feedback[Enable audio feedback sounds (beeps when recording starts/stops)]' \
'(--audio-feedback)--no-audio-feedback[Disable audio feedback sounds]' \
'--pause-media[Pause MPRIS media players during recording]' \
'--duck-media[Lower active media volume during recording instead of pausing it]' \
'--clipboard[Force clipboard mode (don'\''t try to type)]' \
'--paste[Force paste mode (clipboard + Ctrl+V)]' \
'--restore-clipboard[Restore clipboard after paste mode]' \
'--auto-submit[Auto-submit (press Enter) after outputting transcribed text]' \
'(--auto-submit)--no-auto-submit[Disable auto-submit (overrides config auto_submit = true)]' \
'--fallback-to-clipboard[Fall back to clipboard if typing fails]' \
'(--fallback-to-clipboard)--no-fallback-to-clipboard[Disable clipboard fallback]' \
'--wtype-shift-prefix[Prefix wtype output with a Shift key press/release]' \
'--wait-for-modifier-release[Wait for modifier keys (Ctrl/Alt/Shift/Super) to be released before typing]' \
'(--wait-for-modifier-release)--no-wait-for-modifier-release[Disable waiting for modifier release (overrides config)]' \
'--spoken-punctuation[Enable spoken punctuation conversion (e.g., say "period" to get ".")]' \
'--shift-enter-newlines[Convert newlines to Shift+Enter instead of regular Enter]' \
'(--shift-enter-newlines)--no-shift-enter-newlines[Disable Shift+Enter newlines (overrides config)]' \
'--smart-auto-submit[Enable smart auto-submit (say "submit" to press Enter)]' \
'(--smart-auto-submit)--no-smart-auto-submit[Disable smart auto-submit (overrides config)]' \
'--filter-fillers[Filter common filler words ("uh", "um", "er", ...) from transcribed text]' \
'(--filter-fillers)--no-filter-fillers[Disable filler-word filtering (overrides config)]' \
'--vad[Enable Voice Activity Detection (filter silence before transcription)]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
'-V[Print version]' \
'--version[Print version]' \
":: :_voxtype_commands" \
"*::: :->voxtype" \
&& ret=0
    case $state in
    (voxtype)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-command-$line[1]:"
        case $line[1] in
            (daemon)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(transcribe)
_arguments "${_arguments_options[@]}" : \
'--engine=[Override transcription engine]:ENGINE:_default' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':file -- Path to audio file:_files' \
&& ret=0
;;
(transcribe-worker)
_arguments "${_arguments_options[@]}" : \
'--model=[Model name or path (passed from parent process)]:MODEL:_default' \
'--language=[Language code (passed from parent process)]:LANGUAGE:_default' \
'--threads=[Number of threads for inference (passed from parent process)]:THREADS:_default' \
'--translate[Enable translation to English (passed from parent process)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(setup)
_arguments "${_arguments_options[@]}" : \
'--model=[Specify which model to download (use with --download). Whisper\: tiny, base, small, medium, large-v3, large-v3-turbo (and .en variants). Parakeet\: parakeet-tdt-0.6b-v3, parakeet-tdt-0.6b-v3-int8. Other engines (Moonshine, SenseVoice, Paraformer, Dolphin, Omnilingual, Cohere, OpenVINO) take the directory-form names shown by \`voxtype info models\`, e.g. cohere-transcribe-q4f16]:NAME:_default' \
'--progress-format=[How download progress is reported\: human (curl'\''s progress bar) or json (one NDJSON event per update on stdout, for a GUI to render)]:FORMAT:(human json)' \
'--download[Download model if missing (shorthand for basic setup)]' \
'--quiet[Suppress all output (for scripting/automation)]' \
'--no-post-install[Suppress only "Next steps" instructions]' \
'--activate[Also switch the config to use the model, the way the interactive picker does\: sets \`engine\` and \`<engine>.model\`]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
":: :_voxtype__subcmd__setup_commands" \
"*::: :->setup" \
&& ret=0

    case $state in
    (setup)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-setup-command-$line[1]:"
        case $line[1] in
            (check)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(systemd)
_arguments "${_arguments_options[@]}" : \
'--uninstall[Uninstall the service instead of installing]' \
'--status[Show service status]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(waybar)
_arguments "${_arguments_options[@]}" : \
'--json[Output only the JSON config (for scripting)]' \
'--css[Output only the CSS config (for scripting)]' \
'--install[Install waybar integration (inject config and CSS)]' \
'--uninstall[Uninstall waybar integration (remove config and CSS)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(dms)
_arguments "${_arguments_options[@]}" : \
'--install[Install DMS plugin (create widget directory and QML file)]' \
'--uninstall[Uninstall DMS plugin (remove widget directory)]' \
'--qml[Output only the QML content (for scripting)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(model)
_arguments "${_arguments_options[@]}" : \
'--set=[Set a specific model as default (must already be downloaded)]:NAME:_default' \
'--list[List installed models instead of interactive selection]' \
'--restart[Restart the daemon after changing model (use with --set)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(gpu)
_arguments "${_arguments_options[@]}" : \
'--enable[Enable GPU acceleration (auto-detects best backend)]' \
'--disable[Disable GPU acceleration (switch back to CPU)]' \
'--status[Show current backend status]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(npu)
_arguments "${_arguments_options[@]}" : \
'--enable[Enable NPU acceleration and configure OpenVINO]' \
'--disable[Disable NPU acceleration and revert to Whisper]' \
'--status[Show NPU hardware and configuration status]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(variant)
_arguments "${_arguments_options[@]}" : \
'--to=[Variant binary name (e.g., voxtype-avx512, voxtype-onnx-cuda)]:NAME:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(onnx)
_arguments "${_arguments_options[@]}" : \
'--enable[Enable ONNX engine (switch to ONNX binary)]' \
'--disable[Disable ONNX engine (switch back to Whisper binary)]' \
'--status[Show current ONNX backend status]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(parakeet)
_arguments "${_arguments_options[@]}" : \
'--enable[]' \
'--disable[]' \
'--status[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(compositor)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_voxtype__subcmd__setup__subcmd__compositor_commands" \
"*::: :->compositor" \
&& ret=0

    case $state in
    (compositor)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-setup-compositor-command-$line[1]:"
        case $line[1] in
            (hyprland)
_arguments "${_arguments_options[@]}" : \
'--uninstall[Uninstall the compositor integration]' \
'--status[Show installation status]' \
'--show[Show config without installing (print to stdout)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sway)
_arguments "${_arguments_options[@]}" : \
'--uninstall[Uninstall the compositor integration]' \
'--status[Show installation status]' \
'--show[Show config without installing (print to stdout)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(river)
_arguments "${_arguments_options[@]}" : \
'--uninstall[Uninstall the compositor integration]' \
'--status[Show installation status]' \
'--show[Show config without installing (print to stdout)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__setup__subcmd__compositor__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-setup-compositor-help-command-$line[1]:"
        case $line[1] in
            (hyprland)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(sway)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(river)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(vad)
_arguments "${_arguments_options[@]}" : \
'--status[Show VAD model status]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(quickshell)
_arguments "${_arguments_options[@]}" : \
'--target=[Override the install target directory]:DIR:_files' \
'--source=[Override the QML source directory (otherwise auto-detected)]:DIR:_files' \
'--bridge=[Override the source path of the voxtype-audio-bridge binary]:PATH:_files' \
'--bridge-target=[Override the symlink location for voxtype-audio-bridge]:PATH:_files' \
'--force[Overwrite an existing install at the target]' \
'--print-bindings[Skip the file copy; only print the compositor binding examples]' \
'--skip-bridge[Skip installing the voxtype-audio-bridge symlink]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__setup__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-setup-help-command-$line[1]:"
        case $line[1] in
            (check)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(systemd)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(waybar)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(dms)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(model)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(gpu)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(npu)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(variant)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(onnx)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(parakeet)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(compositor)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__setup__subcmd__help__subcmd__compositor_commands" \
"*::: :->compositor" \
&& ret=0

    case $state in
    (compositor)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-setup-help-compositor-command-$line[1]:"
        case $line[1] in
            (hyprland)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(sway)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(river)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(vad)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(quickshell)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(config)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
":: :_voxtype__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-config-command-$line[1]:"
        case $line[1] in
            (set)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':key -- Dotted config key, e.g. hotkey.mode or audio.feedback.volume:_default' \
':value -- New value. Booleans accept true/false, 1/0, yes/no, on/off:_default' \
&& ret=0
;;
(unset)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':key -- Dotted config key to remove:_default' \
&& ret=0
;;
(get)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
'::key -- Dotted config key. Omit to print every allowlisted key:_default' \
&& ret=0
;;
(schema)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__config__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-config-help-command-$line[1]:"
        case $line[1] in
            (set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(unset)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(schema)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(info)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_voxtype__subcmd__info_commands" \
"*::: :->info" \
&& ret=0

    case $state in
    (info)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-info-command-$line[1]:"
        case $line[1] in
            (variants)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(devices)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(models)
_arguments "${_arguments_options[@]}" : \
'--engine=[Restrict output to one engine]:NAME:_default' \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'--verify[Also hash every file of every installed model against the manifest recorded at download time. Thorough and slow\: it reads every byte of every model, which is minutes for a full models directory]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(accel)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(engines)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(styles)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON instead of human-readable text]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__info__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-info-help-command-$line[1]:"
        case $line[1] in
            (variants)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(devices)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(models)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(accel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(engines)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(styles)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(configure)
_arguments "${_arguments_options[@]}" : \
'--force-package-mode[Render as if installed from a package (for testing source builds)]' \
'--probe-audio-devices[Print detected audio input devices (one per line) and exit. Used internally by the TUI to probe devices in a subprocess, so an ALSA hang or crash during probing can'\''t take the TUI with it]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--format=[Output format\: "text" (default) or "json" (for Waybar)]:FORMAT:_default' \
'--icon-theme=[Icon theme for JSON output (emoji, nerd-font, material, phosphor, codicons, omarchy, minimal, dots, arrows, text, or path to custom theme)]:THEME:_default' \
'--follow[Continuously output status changes as JSON (for Waybar exec)]' \
'--extended[Include extended info in JSON (model, device, backend)]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(record)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_voxtype__subcmd__record_commands" \
"*::: :->record" \
&& ret=0

    case $state in
    (record)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-record-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
'--file=[Write transcription to a file Use --file alone to use file_path from config, or --file=path.txt for explicit path]::FILE:_default' \
'--model=[Use a specific model for this transcription (e.g., large-v3-turbo)]:MODEL:_default' \
'--profile=[Use a named profile for post-processing (e.g., --profile slack) Profiles are defined in config.toml under \[profiles.name\]]:NAME:_default' \
'--type[Override output mode to simulate keyboard typing]' \
'--clipboard[Override output mode to clipboard only]' \
'--paste[Override output mode to paste (clipboard + Ctrl+V)]' \
'--auto-submit[Auto-submit (press Enter) after this transcription]' \
'(--auto-submit)--no-auto-submit[Disable auto-submit for this transcription (overrides config)]' \
'--shift-enter-newlines[Use Shift+Enter for newlines in this transcription]' \
'(--shift-enter-newlines)--no-shift-enter-newlines[Disable Shift+Enter newlines for this transcription (overrides config)]' \
'--no-osd[Suppress the on-screen display for this recording only]' \
'(--no-smart-auto-submit)--smart-auto-submit[Enable smart auto-submit for this recording (say "submit" to press Enter)]' \
'(--smart-auto-submit)--no-smart-auto-submit[Disable smart auto-submit for this recording]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
'--timeout=[With --wait, give up after this many seconds]:SECS:_default' \
'--wait-file=[With --wait, the transcript path to wait on]:FILE:_default' \
'--type[Override output mode to simulate keyboard typing]' \
'--clipboard[Override output mode to clipboard only]' \
'--paste[Override output mode to paste (clipboard + Ctrl+V)]' \
'--wait[Block until the transcription is final instead of returning as soon as the daemon has been signalled]' \
'--json[With --wait, print one JSON object describing the outcome]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(toggle)
_arguments "${_arguments_options[@]}" : \
'--file=[Write transcription to a file Use --file alone to use file_path from config, or --file=path.txt for explicit path]::FILE:_default' \
'--model=[Use a specific model for this transcription (e.g., large-v3-turbo)]:MODEL:_default' \
'--profile=[Use a named profile for post-processing (e.g., --profile slack) Profiles are defined in config.toml under \[profiles.name\]]:NAME:_default' \
'--type[Override output mode to simulate keyboard typing]' \
'--clipboard[Override output mode to clipboard only]' \
'--paste[Override output mode to paste (clipboard + Ctrl+V)]' \
'--auto-submit[Auto-submit (press Enter) after this transcription]' \
'(--auto-submit)--no-auto-submit[Disable auto-submit for this transcription (overrides config)]' \
'--shift-enter-newlines[Use Shift+Enter for newlines in this transcription]' \
'(--shift-enter-newlines)--no-shift-enter-newlines[Disable Shift+Enter newlines for this transcription (overrides config)]' \
'--no-osd[Suppress the on-screen display for this recording only]' \
'(--no-smart-auto-submit)--smart-auto-submit[Enable smart auto-submit for this recording (say "submit" to press Enter)]' \
'(--smart-auto-submit)--no-smart-auto-submit[Disable smart auto-submit for this recording (overrides config)]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__record__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-record-help-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(toggle)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(meeting)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
":: :_voxtype__subcmd__meeting_commands" \
"*::: :->meeting" \
&& ret=0

    case $state in
    (meeting)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-meeting-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
'-t+[Meeting title (optional)]:TITLE:_default' \
'--title=[Meeting title (optional)]:TITLE:_default' \
'--diarization=[Diarization backend override for this meeting only]:DIARIZATION:(simple ml)' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(pause)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(resume)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'-l+[Maximum number of meetings to show]:LIMIT:_default' \
'--limit=[Maximum number of meetings to show]:LIMIT:_default' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(export)
_arguments "${_arguments_options[@]}" : \
'-f+[Output format\: text, markdown, json]:FORMAT:_default' \
'--format=[Output format\: text, markdown, json]:FORMAT:_default' \
'-o+[Output file path (default\: stdout)]:OUTPUT:_files' \
'--output=[Output file path (default\: stdout)]:OUTPUT:_files' \
'--timestamps[Include timestamps in output]' \
'--speakers[Include speaker labels in output]' \
'--metadata[Include metadata header in output]' \
'-h[Print help]' \
'--help[Print help]' \
':meeting_id -- Meeting ID (or "latest" for most recent):_default' \
&& ret=0
;;
(show)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':meeting_id -- Meeting ID (or "latest" for most recent):_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'-f[Skip confirmation prompt]' \
'--force[Skip confirmation prompt]' \
'-h[Print help]' \
'--help[Print help]' \
':meeting_id -- Meeting ID:_default' \
&& ret=0
;;
(label)
_arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':meeting_id -- Meeting ID (or "latest" for most recent):_default' \
':speaker_id -- Speaker ID to label (e.g., "SPEAKER_00" or just "0"):_default' \
':label -- Human-readable label to assign:_default' \
&& ret=0
;;
(summarize)
_arguments "${_arguments_options[@]}" : \
'-f+[Output format\: text, json, or markdown]:FORMAT:_default' \
'--format=[Output format\: text, json, or markdown]:FORMAT:_default' \
'-o+[Output file path (default\: stdout)]:OUTPUT:_files' \
'--output=[Output file path (default\: stdout)]:OUTPUT:_files' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':meeting_id -- Meeting ID (or "latest" for most recent):_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__meeting__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-meeting-help-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(pause)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(resume)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(export)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(label)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(summarize)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(check-update)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-help-command-$line[1]:"
        case $line[1] in
            (daemon)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(transcribe)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(transcribe-worker)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(setup)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__help__subcmd__setup_commands" \
"*::: :->setup" \
&& ret=0

    case $state in
    (setup)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-help-setup-command-$line[1]:"
        case $line[1] in
            (check)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(systemd)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(waybar)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(dms)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(model)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(gpu)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(npu)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(variant)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(onnx)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(parakeet)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(compositor)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__help__subcmd__setup__subcmd__compositor_commands" \
"*::: :->compositor" \
&& ret=0

    case $state in
    (compositor)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-help-setup-compositor-command-$line[1]:"
        case $line[1] in
            (hyprland)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(sway)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(river)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(vad)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(quickshell)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(config)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__help__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-help-config-command-$line[1]:"
        case $line[1] in
            (set)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(unset)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(get)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(schema)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(info)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__help__subcmd__info_commands" \
"*::: :->info" \
&& ret=0

    case $state in
    (info)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-help-info-command-$line[1]:"
        case $line[1] in
            (variants)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(devices)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(models)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(accel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(engines)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(styles)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(configure)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(record)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__help__subcmd__record_commands" \
"*::: :->record" \
&& ret=0

    case $state in
    (record)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-help-record-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(toggle)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(meeting)
_arguments "${_arguments_options[@]}" : \
":: :_voxtype__subcmd__help__subcmd__meeting_commands" \
"*::: :->meeting" \
&& ret=0

    case $state in
    (meeting)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:voxtype-help-meeting-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(pause)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(resume)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(export)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(label)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(summarize)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(check-update)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
}

(( $+functions[_voxtype_commands] )) ||
_voxtype_commands() {
    local commands; commands=(
'daemon:Run as daemon (default if no command specified)' \
'transcribe:Transcribe an audio file (WAV, 16kHz, mono)' \
'transcribe-worker:Internal\: Worker process for GPU-isolated transcription Reads audio from stdin, writes transcription result to stdout' \
'setup:Setup and installation utilities' \
'config:Show or modify configuration' \
'info:Inspect runtime/install information' \
'configure:Open the interactive configuration TUI' \
'status:Show daemon status (for Waybar/polybar integration)' \
'record:Control recording from external sources (compositor keybindings, scripts)' \
'meeting:Meeting transcription mode' \
'check-update:Check for updates' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__check-update_commands] )) ||
_voxtype__subcmd__check-update_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype check-update commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config_commands] )) ||
_voxtype__subcmd__config_commands() {
    local commands; commands=(
'set:Set a single configuration value in the on-disk config file' \
'unset:Remove a configuration value, restoring its built-in default' \
'get:Print resolved configuration values' \
'schema:Describe every settable configuration key' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype config commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__get_commands] )) ||
_voxtype__subcmd__config__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config get commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__help_commands] )) ||
_voxtype__subcmd__config__subcmd__help_commands() {
    local commands; commands=(
'set:Set a single configuration value in the on-disk config file' \
'unset:Remove a configuration value, restoring its built-in default' \
'get:Print resolved configuration values' \
'schema:Describe every settable configuration key' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype config help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__help__subcmd__get_commands] )) ||
_voxtype__subcmd__config__subcmd__help__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config help get commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__help__subcmd__help_commands] )) ||
_voxtype__subcmd__config__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config help help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__help__subcmd__schema_commands] )) ||
_voxtype__subcmd__config__subcmd__help__subcmd__schema_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config help schema commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__help__subcmd__set_commands] )) ||
_voxtype__subcmd__config__subcmd__help__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config help set commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__help__subcmd__unset_commands] )) ||
_voxtype__subcmd__config__subcmd__help__subcmd__unset_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config help unset commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__schema_commands] )) ||
_voxtype__subcmd__config__subcmd__schema_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config schema commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__set_commands] )) ||
_voxtype__subcmd__config__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config set commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__config__subcmd__unset_commands] )) ||
_voxtype__subcmd__config__subcmd__unset_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype config unset commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__configure_commands] )) ||
_voxtype__subcmd__configure_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype configure commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__daemon_commands] )) ||
_voxtype__subcmd__daemon_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype daemon commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help_commands] )) ||
_voxtype__subcmd__help_commands() {
    local commands; commands=(
'daemon:Run as daemon (default if no command specified)' \
'transcribe:Transcribe an audio file (WAV, 16kHz, mono)' \
'transcribe-worker:Internal\: Worker process for GPU-isolated transcription Reads audio from stdin, writes transcription result to stdout' \
'setup:Setup and installation utilities' \
'config:Show or modify configuration' \
'info:Inspect runtime/install information' \
'configure:Open the interactive configuration TUI' \
'status:Show daemon status (for Waybar/polybar integration)' \
'record:Control recording from external sources (compositor keybindings, scripts)' \
'meeting:Meeting transcription mode' \
'check-update:Check for updates' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__check-update_commands] )) ||
_voxtype__subcmd__help__subcmd__check-update_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help check-update commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__config_commands] )) ||
_voxtype__subcmd__help__subcmd__config_commands() {
    local commands; commands=(
'set:Set a single configuration value in the on-disk config file' \
'unset:Remove a configuration value, restoring its built-in default' \
'get:Print resolved configuration values' \
'schema:Describe every settable configuration key' \
    )
    _describe -t commands 'voxtype help config commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__config__subcmd__get_commands] )) ||
_voxtype__subcmd__help__subcmd__config__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help config get commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__config__subcmd__schema_commands] )) ||
_voxtype__subcmd__help__subcmd__config__subcmd__schema_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help config schema commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__config__subcmd__set_commands] )) ||
_voxtype__subcmd__help__subcmd__config__subcmd__set_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help config set commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__config__subcmd__unset_commands] )) ||
_voxtype__subcmd__help__subcmd__config__subcmd__unset_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help config unset commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__configure_commands] )) ||
_voxtype__subcmd__help__subcmd__configure_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help configure commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__daemon_commands] )) ||
_voxtype__subcmd__help__subcmd__daemon_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help daemon commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__help_commands] )) ||
_voxtype__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__info_commands] )) ||
_voxtype__subcmd__help__subcmd__info_commands() {
    local commands; commands=(
'variants:Show installed binary variants and which one is active' \
'devices:List audio capture devices' \
'models:List downloadable models per engine and which are installed' \
'accel:Report whether the running daemon is GPU-accelerated' \
'engines:List transcription engines and which are compiled into this binary' \
'styles:List installed OSD styles (Quickshell frontend)' \
    )
    _describe -t commands 'voxtype help info commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__info__subcmd__accel_commands] )) ||
_voxtype__subcmd__help__subcmd__info__subcmd__accel_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help info accel commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__info__subcmd__devices_commands] )) ||
_voxtype__subcmd__help__subcmd__info__subcmd__devices_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help info devices commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__info__subcmd__engines_commands] )) ||
_voxtype__subcmd__help__subcmd__info__subcmd__engines_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help info engines commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__info__subcmd__models_commands] )) ||
_voxtype__subcmd__help__subcmd__info__subcmd__models_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help info models commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__info__subcmd__styles_commands] )) ||
_voxtype__subcmd__help__subcmd__info__subcmd__styles_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help info styles commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__info__subcmd__variants_commands] )) ||
_voxtype__subcmd__help__subcmd__info__subcmd__variants_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help info variants commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting_commands() {
    local commands; commands=(
'start:Start a new meeting transcription' \
'stop:Stop the current meeting' \
'pause:Pause the current meeting' \
'resume:Resume a paused meeting' \
'status:Show meeting status' \
'list:List past meetings' \
'export:Export a meeting transcript' \
'show:Show meeting details' \
'delete:Delete a meeting' \
'label:Label a speaker in a meeting transcript' \
'summarize:Generate an AI summary of a meeting' \
    )
    _describe -t commands 'voxtype help meeting commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__delete_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting delete commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__export_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__export_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting export commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__label_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__label_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting label commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__list_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting list commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__pause_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__pause_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting pause commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__resume_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__resume_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting resume commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__show_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting show commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__start_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting start commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__status_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting status commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__stop_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting stop commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__meeting__subcmd__summarize_commands] )) ||
_voxtype__subcmd__help__subcmd__meeting__subcmd__summarize_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help meeting summarize commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__record_commands] )) ||
_voxtype__subcmd__help__subcmd__record_commands() {
    local commands; commands=(
'start:Start recording (send SIGUSR1 to daemon)' \
'stop:Stop recording and transcribe (send SIGUSR2 to daemon)' \
'toggle:Toggle recording state' \
'cancel:Cancel current recording or transcription (discard without output)' \
    )
    _describe -t commands 'voxtype help record commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__record__subcmd__cancel_commands] )) ||
_voxtype__subcmd__help__subcmd__record__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help record cancel commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__record__subcmd__start_commands] )) ||
_voxtype__subcmd__help__subcmd__record__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help record start commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__record__subcmd__stop_commands] )) ||
_voxtype__subcmd__help__subcmd__record__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help record stop commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__record__subcmd__toggle_commands] )) ||
_voxtype__subcmd__help__subcmd__record__subcmd__toggle_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help record toggle commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup_commands] )) ||
_voxtype__subcmd__help__subcmd__setup_commands() {
    local commands; commands=(
'check:Check system configuration and dependencies' \
'systemd:Install voxtype as a systemd user service (Linux)' \
'waybar:Show Waybar configuration snippets' \
'dms:DankMaterialShell (DMS) integration' \
'model:Interactive model selection and download' \
'gpu:Manage GPU acceleration (Vulkan for Whisper, CUDA/MIGraphX for Parakeet)' \
'npu:Manage Intel NPU acceleration through OpenVINO' \
'variant:Switch the active binary variant (used by \`voxtype configure\` via pkexec)' \
'onnx:Switch between Whisper and ONNX transcription engines' \
'parakeet:Hidden alias for '\''onnx'\'' (backwards compatibility)' \
'compositor:Compositor integration (fixes modifier key interference)' \
'vad:Download the Silero VAD model for speech detection' \
'quickshell:Install the Quickshell QML tree for the voxtype-osd-quickshell launcher' \
    )
    _describe -t commands 'voxtype help setup commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__check_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup check commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__compositor_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__compositor_commands() {
    local commands; commands=(
'hyprland:Hyprland compositor configuration' \
'sway:Sway compositor configuration' \
'river:River compositor configuration' \
    )
    _describe -t commands 'voxtype help setup compositor commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__hyprland_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__hyprland_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup compositor hyprland commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__river_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__river_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup compositor river commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__sway_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__sway_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup compositor sway commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__dms_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__dms_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup dms commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__gpu_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__gpu_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup gpu commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__model_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__model_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup model commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__npu_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__npu_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup npu commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__onnx_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__onnx_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup onnx commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__parakeet_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__parakeet_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup parakeet commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__quickshell_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__quickshell_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup quickshell commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__systemd_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__systemd_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup systemd commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__vad_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__vad_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup vad commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__variant_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__variant_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup variant commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__setup__subcmd__waybar_commands] )) ||
_voxtype__subcmd__help__subcmd__setup__subcmd__waybar_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help setup waybar commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__status_commands] )) ||
_voxtype__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help status commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__transcribe_commands] )) ||
_voxtype__subcmd__help__subcmd__transcribe_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help transcribe commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__help__subcmd__transcribe-worker_commands] )) ||
_voxtype__subcmd__help__subcmd__transcribe-worker_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype help transcribe-worker commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info_commands] )) ||
_voxtype__subcmd__info_commands() {
    local commands; commands=(
'variants:Show installed binary variants and which one is active' \
'devices:List audio capture devices' \
'models:List downloadable models per engine and which are installed' \
'accel:Report whether the running daemon is GPU-accelerated' \
'engines:List transcription engines and which are compiled into this binary' \
'styles:List installed OSD styles (Quickshell frontend)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype info commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__accel_commands] )) ||
_voxtype__subcmd__info__subcmd__accel_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info accel commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__devices_commands] )) ||
_voxtype__subcmd__info__subcmd__devices_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info devices commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__engines_commands] )) ||
_voxtype__subcmd__info__subcmd__engines_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info engines commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help_commands] )) ||
_voxtype__subcmd__info__subcmd__help_commands() {
    local commands; commands=(
'variants:Show installed binary variants and which one is active' \
'devices:List audio capture devices' \
'models:List downloadable models per engine and which are installed' \
'accel:Report whether the running daemon is GPU-accelerated' \
'engines:List transcription engines and which are compiled into this binary' \
'styles:List installed OSD styles (Quickshell frontend)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype info help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help__subcmd__accel_commands] )) ||
_voxtype__subcmd__info__subcmd__help__subcmd__accel_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info help accel commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help__subcmd__devices_commands] )) ||
_voxtype__subcmd__info__subcmd__help__subcmd__devices_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info help devices commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help__subcmd__engines_commands] )) ||
_voxtype__subcmd__info__subcmd__help__subcmd__engines_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info help engines commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help__subcmd__help_commands] )) ||
_voxtype__subcmd__info__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info help help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help__subcmd__models_commands] )) ||
_voxtype__subcmd__info__subcmd__help__subcmd__models_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info help models commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help__subcmd__styles_commands] )) ||
_voxtype__subcmd__info__subcmd__help__subcmd__styles_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info help styles commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__help__subcmd__variants_commands] )) ||
_voxtype__subcmd__info__subcmd__help__subcmd__variants_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info help variants commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__models_commands] )) ||
_voxtype__subcmd__info__subcmd__models_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info models commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__styles_commands] )) ||
_voxtype__subcmd__info__subcmd__styles_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info styles commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__info__subcmd__variants_commands] )) ||
_voxtype__subcmd__info__subcmd__variants_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype info variants commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting_commands] )) ||
_voxtype__subcmd__meeting_commands() {
    local commands; commands=(
'start:Start a new meeting transcription' \
'stop:Stop the current meeting' \
'pause:Pause the current meeting' \
'resume:Resume a paused meeting' \
'status:Show meeting status' \
'list:List past meetings' \
'export:Export a meeting transcript' \
'show:Show meeting details' \
'delete:Delete a meeting' \
'label:Label a speaker in a meeting transcript' \
'summarize:Generate an AI summary of a meeting' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype meeting commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__delete_commands] )) ||
_voxtype__subcmd__meeting__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting delete commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__export_commands] )) ||
_voxtype__subcmd__meeting__subcmd__export_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting export commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help_commands() {
    local commands; commands=(
'start:Start a new meeting transcription' \
'stop:Stop the current meeting' \
'pause:Pause the current meeting' \
'resume:Resume a paused meeting' \
'status:Show meeting status' \
'list:List past meetings' \
'export:Export a meeting transcript' \
'show:Show meeting details' \
'delete:Delete a meeting' \
'label:Label a speaker in a meeting transcript' \
'summarize:Generate an AI summary of a meeting' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype meeting help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__delete_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help delete commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__export_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__export_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help export commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__help_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__label_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__label_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help label commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__list_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help list commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__pause_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__pause_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help pause commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__resume_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__resume_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help resume commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__show_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help show commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__start_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help start commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__status_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help status commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__stop_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help stop commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__help__subcmd__summarize_commands] )) ||
_voxtype__subcmd__meeting__subcmd__help__subcmd__summarize_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting help summarize commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__label_commands] )) ||
_voxtype__subcmd__meeting__subcmd__label_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting label commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__list_commands] )) ||
_voxtype__subcmd__meeting__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting list commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__pause_commands] )) ||
_voxtype__subcmd__meeting__subcmd__pause_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting pause commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__resume_commands] )) ||
_voxtype__subcmd__meeting__subcmd__resume_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting resume commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__show_commands] )) ||
_voxtype__subcmd__meeting__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting show commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__start_commands] )) ||
_voxtype__subcmd__meeting__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting start commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__status_commands] )) ||
_voxtype__subcmd__meeting__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting status commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__stop_commands] )) ||
_voxtype__subcmd__meeting__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting stop commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__meeting__subcmd__summarize_commands] )) ||
_voxtype__subcmd__meeting__subcmd__summarize_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype meeting summarize commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record_commands] )) ||
_voxtype__subcmd__record_commands() {
    local commands; commands=(
'start:Start recording (send SIGUSR1 to daemon)' \
'stop:Stop recording and transcribe (send SIGUSR2 to daemon)' \
'toggle:Toggle recording state' \
'cancel:Cancel current recording or transcription (discard without output)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype record commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__cancel_commands] )) ||
_voxtype__subcmd__record__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record cancel commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__help_commands] )) ||
_voxtype__subcmd__record__subcmd__help_commands() {
    local commands; commands=(
'start:Start recording (send SIGUSR1 to daemon)' \
'stop:Stop recording and transcribe (send SIGUSR2 to daemon)' \
'toggle:Toggle recording state' \
'cancel:Cancel current recording or transcription (discard without output)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype record help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__help__subcmd__cancel_commands] )) ||
_voxtype__subcmd__record__subcmd__help__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record help cancel commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__help__subcmd__help_commands] )) ||
_voxtype__subcmd__record__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record help help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__help__subcmd__start_commands] )) ||
_voxtype__subcmd__record__subcmd__help__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record help start commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__help__subcmd__stop_commands] )) ||
_voxtype__subcmd__record__subcmd__help__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record help stop commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__help__subcmd__toggle_commands] )) ||
_voxtype__subcmd__record__subcmd__help__subcmd__toggle_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record help toggle commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__start_commands] )) ||
_voxtype__subcmd__record__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record start commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__stop_commands] )) ||
_voxtype__subcmd__record__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record stop commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__record__subcmd__toggle_commands] )) ||
_voxtype__subcmd__record__subcmd__toggle_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype record toggle commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup_commands] )) ||
_voxtype__subcmd__setup_commands() {
    local commands; commands=(
'check:Check system configuration and dependencies' \
'systemd:Install voxtype as a systemd user service (Linux)' \
'waybar:Show Waybar configuration snippets' \
'dms:DankMaterialShell (DMS) integration' \
'model:Interactive model selection and download' \
'gpu:Manage GPU acceleration (Vulkan for Whisper, CUDA/MIGraphX for Parakeet)' \
'npu:Manage Intel NPU acceleration through OpenVINO' \
'variant:Switch the active binary variant (used by \`voxtype configure\` via pkexec)' \
'onnx:Switch between Whisper and ONNX transcription engines' \
'parakeet:Hidden alias for '\''onnx'\'' (backwards compatibility)' \
'compositor:Compositor integration (fixes modifier key interference)' \
'vad:Download the Silero VAD model for speech detection' \
'quickshell:Install the Quickshell QML tree for the voxtype-osd-quickshell launcher' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype setup commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__check_commands] )) ||
_voxtype__subcmd__setup__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup check commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor_commands() {
    local commands; commands=(
'hyprland:Hyprland compositor configuration' \
'sway:Sway compositor configuration' \
'river:River compositor configuration' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype setup compositor commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__help_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__help_commands() {
    local commands; commands=(
'hyprland:Hyprland compositor configuration' \
'sway:Sway compositor configuration' \
'river:River compositor configuration' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype setup compositor help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__help_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup compositor help help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__hyprland_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__hyprland_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup compositor help hyprland commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__river_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__river_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup compositor help river commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__sway_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__sway_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup compositor help sway commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__hyprland_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__hyprland_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup compositor hyprland commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__river_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__river_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup compositor river commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__compositor__subcmd__sway_commands] )) ||
_voxtype__subcmd__setup__subcmd__compositor__subcmd__sway_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup compositor sway commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__dms_commands] )) ||
_voxtype__subcmd__setup__subcmd__dms_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup dms commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__gpu_commands] )) ||
_voxtype__subcmd__setup__subcmd__gpu_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup gpu commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help_commands] )) ||
_voxtype__subcmd__setup__subcmd__help_commands() {
    local commands; commands=(
'check:Check system configuration and dependencies' \
'systemd:Install voxtype as a systemd user service (Linux)' \
'waybar:Show Waybar configuration snippets' \
'dms:DankMaterialShell (DMS) integration' \
'model:Interactive model selection and download' \
'gpu:Manage GPU acceleration (Vulkan for Whisper, CUDA/MIGraphX for Parakeet)' \
'npu:Manage Intel NPU acceleration through OpenVINO' \
'variant:Switch the active binary variant (used by \`voxtype configure\` via pkexec)' \
'onnx:Switch between Whisper and ONNX transcription engines' \
'parakeet:Hidden alias for '\''onnx'\'' (backwards compatibility)' \
'compositor:Compositor integration (fixes modifier key interference)' \
'vad:Download the Silero VAD model for speech detection' \
'quickshell:Install the Quickshell QML tree for the voxtype-osd-quickshell launcher' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'voxtype setup help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__check_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help check commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__compositor_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__compositor_commands() {
    local commands; commands=(
'hyprland:Hyprland compositor configuration' \
'sway:Sway compositor configuration' \
'river:River compositor configuration' \
    )
    _describe -t commands 'voxtype setup help compositor commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__hyprland_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__hyprland_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help compositor hyprland commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__river_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__river_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help compositor river commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__sway_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__sway_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help compositor sway commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__dms_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__dms_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help dms commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__gpu_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__gpu_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help gpu commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__help_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help help commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__model_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__model_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help model commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__npu_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__npu_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help npu commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__onnx_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__onnx_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help onnx commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__parakeet_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__parakeet_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help parakeet commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__quickshell_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__quickshell_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help quickshell commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__systemd_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__systemd_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help systemd commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__vad_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__vad_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help vad commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__variant_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__variant_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help variant commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__help__subcmd__waybar_commands] )) ||
_voxtype__subcmd__setup__subcmd__help__subcmd__waybar_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup help waybar commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__model_commands] )) ||
_voxtype__subcmd__setup__subcmd__model_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup model commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__npu_commands] )) ||
_voxtype__subcmd__setup__subcmd__npu_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup npu commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__onnx_commands] )) ||
_voxtype__subcmd__setup__subcmd__onnx_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup onnx commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__parakeet_commands] )) ||
_voxtype__subcmd__setup__subcmd__parakeet_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup parakeet commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__quickshell_commands] )) ||
_voxtype__subcmd__setup__subcmd__quickshell_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup quickshell commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__systemd_commands] )) ||
_voxtype__subcmd__setup__subcmd__systemd_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup systemd commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__vad_commands] )) ||
_voxtype__subcmd__setup__subcmd__vad_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup vad commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__variant_commands] )) ||
_voxtype__subcmd__setup__subcmd__variant_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup variant commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__setup__subcmd__waybar_commands] )) ||
_voxtype__subcmd__setup__subcmd__waybar_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype setup waybar commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__status_commands] )) ||
_voxtype__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype status commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__transcribe_commands] )) ||
_voxtype__subcmd__transcribe_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype transcribe commands' commands "$@"
}
(( $+functions[_voxtype__subcmd__transcribe-worker_commands] )) ||
_voxtype__subcmd__transcribe-worker_commands() {
    local commands; commands=()
    _describe -t commands 'voxtype transcribe-worker commands' commands "$@"
}

if [ "$funcstack[1]" = "_voxtype" ]; then
    _voxtype "$@"
else
    compdef _voxtype voxtype
fi

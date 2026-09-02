#compdef voxtype

_voxtype() {
    local -a commands
    commands=(
        'daemon:Run as background daemon'
        'transcribe:Transcribe an audio file'
        'setup:Check dependencies and download models'
        'config:Show current configuration'
        'help:Print help information'
    )

    local -a global_opts
    global_opts=(
        '(-c --config)'{-c,--config}'[Path to config file]:config file:_files -g "*.toml"'
        '(-v --verbose)'{-v,--verbose}'[Increase verbosity (-v, -vv)]'
        '(-q --quiet)'{-q,--quiet}'[Quiet mode (errors only)]'
        '--clipboard[Force clipboard mode]'
        '--model[Override whisper model]:model:(tiny tiny.en base base.en small small.en medium medium.en large-v3)'
        '--hotkey[Override hotkey]:key:(SCROLLLOCK PAUSE RIGHTALT F13 F14 F15 F16 F17 F18 F19 F20 F21 F22 F23 F24)'
        '--hotkey-backend[Override Linux hotkey backend]:backend:(evdev portal auto)'
        '(-h --help)'{-h,--help}'[Print help]'
        '(-V --version)'{-V,--version}'[Print version]'
    )

    _arguments -C \
        $global_opts \
        '1:command:->command' \
        '*::arg:->args'

    case $state in
        command)
            _describe -t commands 'voxtype command' commands
            ;;
        args)
            case $words[1] in
                transcribe)
                    _arguments \
                        '(-o --output)'{-o,--output}'[Output file]:output file:_files' \
                        '--model[Override whisper model]:model:(tiny tiny.en base base.en small small.en medium medium.en large-v3)' \
                        '(-h --help)'{-h,--help}'[Print help]' \
                        '*:audio file:_files -g "*.wav *.mp3 *.flac *.ogg *.m4a"'
                    ;;
                setup)
                    _arguments \
                        '--download[Download whisper model]' \
                        '--model[Model to download]:model:(tiny tiny.en base base.en small small.en medium medium.en large-v3)' \
                        '(-h --help)'{-h,--help}'[Print help]'
                    ;;
                daemon|config)
                    _arguments \
                        '(-h --help)'{-h,--help}'[Print help]'
                    ;;
            esac
            ;;
    esac
}

_voxtype "$@"

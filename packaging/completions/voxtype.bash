_voxtype() {
    local cur prev words cword
    _init_completion || return

    local commands="daemon transcribe setup config help"
    local global_opts="-c --config -v --verbose -q --quiet --clipboard --model --hotkey --hotkey-backend -h --help -V --version"

    case $prev in
        -c|--config)
            _filedir toml
            return
            ;;
        --model)
            COMPREPLY=($(compgen -W "tiny tiny.en base base.en small small.en medium medium.en large-v3" -- "$cur"))
            return
            ;;
        --hotkey)
            COMPREPLY=($(compgen -W "SCROLLLOCK PAUSE RIGHTALT F13 F14 F15 F16 F17 F18 F19 F20 F21 F22 F23 F24" -- "$cur"))
            return
            ;;
        --hotkey-backend)
            COMPREPLY=($(compgen -W "evdev portal auto" -- "$cur"))
            return
            ;;
        transcribe)
            _filedir '@(wav|mp3|flac|ogg|m4a)'
            return
            ;;
    esac

    case ${words[1]} in
        daemon)
            COMPREPLY=($(compgen -W "$global_opts" -- "$cur"))
            return
            ;;
        transcribe)
            if [[ $cur == -* ]]; then
                COMPREPLY=($(compgen -W "-o --output --model -h --help" -- "$cur"))
            else
                _filedir '@(wav|mp3|flac|ogg|m4a)'
            fi
            return
            ;;
        setup)
            COMPREPLY=($(compgen -W "--download --model -h --help" -- "$cur"))
            return
            ;;
        config)
            COMPREPLY=($(compgen -W "-h --help" -- "$cur"))
            return
            ;;
    esac

    if [[ $cur == -* ]]; then
        COMPREPLY=($(compgen -W "$global_opts" -- "$cur"))
    else
        COMPREPLY=($(compgen -W "$commands" -- "$cur"))
    fi
}

complete -F _voxtype voxtype

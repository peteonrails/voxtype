_voxtype() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="voxtype"
                ;;
            voxtype,check-update)
                cmd="voxtype__subcmd__check__subcmd__update"
                ;;
            voxtype,config)
                cmd="voxtype__subcmd__config"
                ;;
            voxtype,configure)
                cmd="voxtype__subcmd__configure"
                ;;
            voxtype,daemon)
                cmd="voxtype__subcmd__daemon"
                ;;
            voxtype,help)
                cmd="voxtype__subcmd__help"
                ;;
            voxtype,info)
                cmd="voxtype__subcmd__info"
                ;;
            voxtype,meeting)
                cmd="voxtype__subcmd__meeting"
                ;;
            voxtype,record)
                cmd="voxtype__subcmd__record"
                ;;
            voxtype,setup)
                cmd="voxtype__subcmd__setup"
                ;;
            voxtype,status)
                cmd="voxtype__subcmd__status"
                ;;
            voxtype,transcribe)
                cmd="voxtype__subcmd__transcribe"
                ;;
            voxtype,transcribe-worker)
                cmd="voxtype__subcmd__transcribe__subcmd__worker"
                ;;
            voxtype__subcmd__config,get)
                cmd="voxtype__subcmd__config__subcmd__get"
                ;;
            voxtype__subcmd__config,help)
                cmd="voxtype__subcmd__config__subcmd__help"
                ;;
            voxtype__subcmd__config,schema)
                cmd="voxtype__subcmd__config__subcmd__schema"
                ;;
            voxtype__subcmd__config,set)
                cmd="voxtype__subcmd__config__subcmd__set"
                ;;
            voxtype__subcmd__config,unset)
                cmd="voxtype__subcmd__config__subcmd__unset"
                ;;
            voxtype__subcmd__config__subcmd__help,get)
                cmd="voxtype__subcmd__config__subcmd__help__subcmd__get"
                ;;
            voxtype__subcmd__config__subcmd__help,help)
                cmd="voxtype__subcmd__config__subcmd__help__subcmd__help"
                ;;
            voxtype__subcmd__config__subcmd__help,schema)
                cmd="voxtype__subcmd__config__subcmd__help__subcmd__schema"
                ;;
            voxtype__subcmd__config__subcmd__help,set)
                cmd="voxtype__subcmd__config__subcmd__help__subcmd__set"
                ;;
            voxtype__subcmd__config__subcmd__help,unset)
                cmd="voxtype__subcmd__config__subcmd__help__subcmd__unset"
                ;;
            voxtype__subcmd__help,check-update)
                cmd="voxtype__subcmd__help__subcmd__check__subcmd__update"
                ;;
            voxtype__subcmd__help,config)
                cmd="voxtype__subcmd__help__subcmd__config"
                ;;
            voxtype__subcmd__help,configure)
                cmd="voxtype__subcmd__help__subcmd__configure"
                ;;
            voxtype__subcmd__help,daemon)
                cmd="voxtype__subcmd__help__subcmd__daemon"
                ;;
            voxtype__subcmd__help,help)
                cmd="voxtype__subcmd__help__subcmd__help"
                ;;
            voxtype__subcmd__help,info)
                cmd="voxtype__subcmd__help__subcmd__info"
                ;;
            voxtype__subcmd__help,meeting)
                cmd="voxtype__subcmd__help__subcmd__meeting"
                ;;
            voxtype__subcmd__help,record)
                cmd="voxtype__subcmd__help__subcmd__record"
                ;;
            voxtype__subcmd__help,setup)
                cmd="voxtype__subcmd__help__subcmd__setup"
                ;;
            voxtype__subcmd__help,status)
                cmd="voxtype__subcmd__help__subcmd__status"
                ;;
            voxtype__subcmd__help,transcribe)
                cmd="voxtype__subcmd__help__subcmd__transcribe"
                ;;
            voxtype__subcmd__help,transcribe-worker)
                cmd="voxtype__subcmd__help__subcmd__transcribe__subcmd__worker"
                ;;
            voxtype__subcmd__help__subcmd__config,get)
                cmd="voxtype__subcmd__help__subcmd__config__subcmd__get"
                ;;
            voxtype__subcmd__help__subcmd__config,schema)
                cmd="voxtype__subcmd__help__subcmd__config__subcmd__schema"
                ;;
            voxtype__subcmd__help__subcmd__config,set)
                cmd="voxtype__subcmd__help__subcmd__config__subcmd__set"
                ;;
            voxtype__subcmd__help__subcmd__config,unset)
                cmd="voxtype__subcmd__help__subcmd__config__subcmd__unset"
                ;;
            voxtype__subcmd__help__subcmd__info,accel)
                cmd="voxtype__subcmd__help__subcmd__info__subcmd__accel"
                ;;
            voxtype__subcmd__help__subcmd__info,devices)
                cmd="voxtype__subcmd__help__subcmd__info__subcmd__devices"
                ;;
            voxtype__subcmd__help__subcmd__info,engines)
                cmd="voxtype__subcmd__help__subcmd__info__subcmd__engines"
                ;;
            voxtype__subcmd__help__subcmd__info,models)
                cmd="voxtype__subcmd__help__subcmd__info__subcmd__models"
                ;;
            voxtype__subcmd__help__subcmd__info,styles)
                cmd="voxtype__subcmd__help__subcmd__info__subcmd__styles"
                ;;
            voxtype__subcmd__help__subcmd__info,variants)
                cmd="voxtype__subcmd__help__subcmd__info__subcmd__variants"
                ;;
            voxtype__subcmd__help__subcmd__meeting,delete)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__delete"
                ;;
            voxtype__subcmd__help__subcmd__meeting,export)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__export"
                ;;
            voxtype__subcmd__help__subcmd__meeting,label)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__label"
                ;;
            voxtype__subcmd__help__subcmd__meeting,list)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__list"
                ;;
            voxtype__subcmd__help__subcmd__meeting,pause)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__pause"
                ;;
            voxtype__subcmd__help__subcmd__meeting,resume)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__resume"
                ;;
            voxtype__subcmd__help__subcmd__meeting,show)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__show"
                ;;
            voxtype__subcmd__help__subcmd__meeting,start)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__start"
                ;;
            voxtype__subcmd__help__subcmd__meeting,status)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__status"
                ;;
            voxtype__subcmd__help__subcmd__meeting,stop)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__stop"
                ;;
            voxtype__subcmd__help__subcmd__meeting,summarize)
                cmd="voxtype__subcmd__help__subcmd__meeting__subcmd__summarize"
                ;;
            voxtype__subcmd__help__subcmd__record,cancel)
                cmd="voxtype__subcmd__help__subcmd__record__subcmd__cancel"
                ;;
            voxtype__subcmd__help__subcmd__record,start)
                cmd="voxtype__subcmd__help__subcmd__record__subcmd__start"
                ;;
            voxtype__subcmd__help__subcmd__record,stop)
                cmd="voxtype__subcmd__help__subcmd__record__subcmd__stop"
                ;;
            voxtype__subcmd__help__subcmd__record,toggle)
                cmd="voxtype__subcmd__help__subcmd__record__subcmd__toggle"
                ;;
            voxtype__subcmd__help__subcmd__setup,check)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__check"
                ;;
            voxtype__subcmd__help__subcmd__setup,compositor)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__compositor"
                ;;
            voxtype__subcmd__help__subcmd__setup,dms)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__dms"
                ;;
            voxtype__subcmd__help__subcmd__setup,gpu)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__gpu"
                ;;
            voxtype__subcmd__help__subcmd__setup,model)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__model"
                ;;
            voxtype__subcmd__help__subcmd__setup,npu)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__npu"
                ;;
            voxtype__subcmd__help__subcmd__setup,onnx)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__onnx"
                ;;
            voxtype__subcmd__help__subcmd__setup,parakeet)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__parakeet"
                ;;
            voxtype__subcmd__help__subcmd__setup,quickshell)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__quickshell"
                ;;
            voxtype__subcmd__help__subcmd__setup,systemd)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__systemd"
                ;;
            voxtype__subcmd__help__subcmd__setup,vad)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__vad"
                ;;
            voxtype__subcmd__help__subcmd__setup,variant)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__variant"
                ;;
            voxtype__subcmd__help__subcmd__setup,waybar)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__waybar"
                ;;
            voxtype__subcmd__help__subcmd__setup__subcmd__compositor,hyprland)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__hyprland"
                ;;
            voxtype__subcmd__help__subcmd__setup__subcmd__compositor,river)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__river"
                ;;
            voxtype__subcmd__help__subcmd__setup__subcmd__compositor,sway)
                cmd="voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__sway"
                ;;
            voxtype__subcmd__info,accel)
                cmd="voxtype__subcmd__info__subcmd__accel"
                ;;
            voxtype__subcmd__info,devices)
                cmd="voxtype__subcmd__info__subcmd__devices"
                ;;
            voxtype__subcmd__info,engines)
                cmd="voxtype__subcmd__info__subcmd__engines"
                ;;
            voxtype__subcmd__info,help)
                cmd="voxtype__subcmd__info__subcmd__help"
                ;;
            voxtype__subcmd__info,models)
                cmd="voxtype__subcmd__info__subcmd__models"
                ;;
            voxtype__subcmd__info,styles)
                cmd="voxtype__subcmd__info__subcmd__styles"
                ;;
            voxtype__subcmd__info,variants)
                cmd="voxtype__subcmd__info__subcmd__variants"
                ;;
            voxtype__subcmd__info__subcmd__help,accel)
                cmd="voxtype__subcmd__info__subcmd__help__subcmd__accel"
                ;;
            voxtype__subcmd__info__subcmd__help,devices)
                cmd="voxtype__subcmd__info__subcmd__help__subcmd__devices"
                ;;
            voxtype__subcmd__info__subcmd__help,engines)
                cmd="voxtype__subcmd__info__subcmd__help__subcmd__engines"
                ;;
            voxtype__subcmd__info__subcmd__help,help)
                cmd="voxtype__subcmd__info__subcmd__help__subcmd__help"
                ;;
            voxtype__subcmd__info__subcmd__help,models)
                cmd="voxtype__subcmd__info__subcmd__help__subcmd__models"
                ;;
            voxtype__subcmd__info__subcmd__help,styles)
                cmd="voxtype__subcmd__info__subcmd__help__subcmd__styles"
                ;;
            voxtype__subcmd__info__subcmd__help,variants)
                cmd="voxtype__subcmd__info__subcmd__help__subcmd__variants"
                ;;
            voxtype__subcmd__meeting,delete)
                cmd="voxtype__subcmd__meeting__subcmd__delete"
                ;;
            voxtype__subcmd__meeting,export)
                cmd="voxtype__subcmd__meeting__subcmd__export"
                ;;
            voxtype__subcmd__meeting,help)
                cmd="voxtype__subcmd__meeting__subcmd__help"
                ;;
            voxtype__subcmd__meeting,label)
                cmd="voxtype__subcmd__meeting__subcmd__label"
                ;;
            voxtype__subcmd__meeting,list)
                cmd="voxtype__subcmd__meeting__subcmd__list"
                ;;
            voxtype__subcmd__meeting,pause)
                cmd="voxtype__subcmd__meeting__subcmd__pause"
                ;;
            voxtype__subcmd__meeting,resume)
                cmd="voxtype__subcmd__meeting__subcmd__resume"
                ;;
            voxtype__subcmd__meeting,show)
                cmd="voxtype__subcmd__meeting__subcmd__show"
                ;;
            voxtype__subcmd__meeting,start)
                cmd="voxtype__subcmd__meeting__subcmd__start"
                ;;
            voxtype__subcmd__meeting,status)
                cmd="voxtype__subcmd__meeting__subcmd__status"
                ;;
            voxtype__subcmd__meeting,stop)
                cmd="voxtype__subcmd__meeting__subcmd__stop"
                ;;
            voxtype__subcmd__meeting,summarize)
                cmd="voxtype__subcmd__meeting__subcmd__summarize"
                ;;
            voxtype__subcmd__meeting__subcmd__help,delete)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__delete"
                ;;
            voxtype__subcmd__meeting__subcmd__help,export)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__export"
                ;;
            voxtype__subcmd__meeting__subcmd__help,help)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__help"
                ;;
            voxtype__subcmd__meeting__subcmd__help,label)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__label"
                ;;
            voxtype__subcmd__meeting__subcmd__help,list)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__list"
                ;;
            voxtype__subcmd__meeting__subcmd__help,pause)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__pause"
                ;;
            voxtype__subcmd__meeting__subcmd__help,resume)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__resume"
                ;;
            voxtype__subcmd__meeting__subcmd__help,show)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__show"
                ;;
            voxtype__subcmd__meeting__subcmd__help,start)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__start"
                ;;
            voxtype__subcmd__meeting__subcmd__help,status)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__status"
                ;;
            voxtype__subcmd__meeting__subcmd__help,stop)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__stop"
                ;;
            voxtype__subcmd__meeting__subcmd__help,summarize)
                cmd="voxtype__subcmd__meeting__subcmd__help__subcmd__summarize"
                ;;
            voxtype__subcmd__record,cancel)
                cmd="voxtype__subcmd__record__subcmd__cancel"
                ;;
            voxtype__subcmd__record,help)
                cmd="voxtype__subcmd__record__subcmd__help"
                ;;
            voxtype__subcmd__record,start)
                cmd="voxtype__subcmd__record__subcmd__start"
                ;;
            voxtype__subcmd__record,stop)
                cmd="voxtype__subcmd__record__subcmd__stop"
                ;;
            voxtype__subcmd__record,toggle)
                cmd="voxtype__subcmd__record__subcmd__toggle"
                ;;
            voxtype__subcmd__record__subcmd__help,cancel)
                cmd="voxtype__subcmd__record__subcmd__help__subcmd__cancel"
                ;;
            voxtype__subcmd__record__subcmd__help,help)
                cmd="voxtype__subcmd__record__subcmd__help__subcmd__help"
                ;;
            voxtype__subcmd__record__subcmd__help,start)
                cmd="voxtype__subcmd__record__subcmd__help__subcmd__start"
                ;;
            voxtype__subcmd__record__subcmd__help,stop)
                cmd="voxtype__subcmd__record__subcmd__help__subcmd__stop"
                ;;
            voxtype__subcmd__record__subcmd__help,toggle)
                cmd="voxtype__subcmd__record__subcmd__help__subcmd__toggle"
                ;;
            voxtype__subcmd__setup,check)
                cmd="voxtype__subcmd__setup__subcmd__check"
                ;;
            voxtype__subcmd__setup,compositor)
                cmd="voxtype__subcmd__setup__subcmd__compositor"
                ;;
            voxtype__subcmd__setup,dms)
                cmd="voxtype__subcmd__setup__subcmd__dms"
                ;;
            voxtype__subcmd__setup,gpu)
                cmd="voxtype__subcmd__setup__subcmd__gpu"
                ;;
            voxtype__subcmd__setup,help)
                cmd="voxtype__subcmd__setup__subcmd__help"
                ;;
            voxtype__subcmd__setup,model)
                cmd="voxtype__subcmd__setup__subcmd__model"
                ;;
            voxtype__subcmd__setup,npu)
                cmd="voxtype__subcmd__setup__subcmd__npu"
                ;;
            voxtype__subcmd__setup,onnx)
                cmd="voxtype__subcmd__setup__subcmd__onnx"
                ;;
            voxtype__subcmd__setup,parakeet)
                cmd="voxtype__subcmd__setup__subcmd__parakeet"
                ;;
            voxtype__subcmd__setup,quickshell)
                cmd="voxtype__subcmd__setup__subcmd__quickshell"
                ;;
            voxtype__subcmd__setup,systemd)
                cmd="voxtype__subcmd__setup__subcmd__systemd"
                ;;
            voxtype__subcmd__setup,vad)
                cmd="voxtype__subcmd__setup__subcmd__vad"
                ;;
            voxtype__subcmd__setup,variant)
                cmd="voxtype__subcmd__setup__subcmd__variant"
                ;;
            voxtype__subcmd__setup,waybar)
                cmd="voxtype__subcmd__setup__subcmd__waybar"
                ;;
            voxtype__subcmd__setup__subcmd__compositor,help)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__help"
                ;;
            voxtype__subcmd__setup__subcmd__compositor,hyprland)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__hyprland"
                ;;
            voxtype__subcmd__setup__subcmd__compositor,river)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__river"
                ;;
            voxtype__subcmd__setup__subcmd__compositor,sway)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__sway"
                ;;
            voxtype__subcmd__setup__subcmd__compositor__subcmd__help,help)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__help"
                ;;
            voxtype__subcmd__setup__subcmd__compositor__subcmd__help,hyprland)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__hyprland"
                ;;
            voxtype__subcmd__setup__subcmd__compositor__subcmd__help,river)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__river"
                ;;
            voxtype__subcmd__setup__subcmd__compositor__subcmd__help,sway)
                cmd="voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__sway"
                ;;
            voxtype__subcmd__setup__subcmd__help,check)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__check"
                ;;
            voxtype__subcmd__setup__subcmd__help,compositor)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__compositor"
                ;;
            voxtype__subcmd__setup__subcmd__help,dms)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__dms"
                ;;
            voxtype__subcmd__setup__subcmd__help,gpu)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__gpu"
                ;;
            voxtype__subcmd__setup__subcmd__help,help)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__help"
                ;;
            voxtype__subcmd__setup__subcmd__help,model)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__model"
                ;;
            voxtype__subcmd__setup__subcmd__help,npu)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__npu"
                ;;
            voxtype__subcmd__setup__subcmd__help,onnx)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__onnx"
                ;;
            voxtype__subcmd__setup__subcmd__help,parakeet)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__parakeet"
                ;;
            voxtype__subcmd__setup__subcmd__help,quickshell)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__quickshell"
                ;;
            voxtype__subcmd__setup__subcmd__help,systemd)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__systemd"
                ;;
            voxtype__subcmd__setup__subcmd__help,vad)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__vad"
                ;;
            voxtype__subcmd__setup__subcmd__help,variant)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__variant"
                ;;
            voxtype__subcmd__setup__subcmd__help,waybar)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__waybar"
                ;;
            voxtype__subcmd__setup__subcmd__help__subcmd__compositor,hyprland)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__hyprland"
                ;;
            voxtype__subcmd__setup__subcmd__help__subcmd__compositor,river)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__river"
                ;;
            voxtype__subcmd__setup__subcmd__help__subcmd__compositor,sway)
                cmd="voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__sway"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        voxtype)
            opts="-c -v -q -h -V --config --verbose --quiet --model --engine --language --translate --threads --gpu-isolation --gpu-device --on-demand-loading --secondary-model --eager-processing --no-whisper-context-optimization --initial-prompt --flash-attention --whisper-mode --remote-endpoint --remote-model --remote-api-key --soniox-api-key --hotkey --toggle --no-hotkey --cancel-key --model-modifier --audio-device --max-duration --audio-feedback --no-audio-feedback --pause-media --duck-media --duck-media-volume --duck-media-fade-ms --clipboard --paste --restore-clipboard --restore-clipboard-delay-ms --driver --auto-submit --no-auto-submit --fallback-to-clipboard --no-fallback-to-clipboard --paste-keys --file-path --file-mode --pre-type-delay --wtype-delay --wtype-shift-prefix --type-delay --dotool-xkb-layout --dotool-xkb-variant --eitype-xkb-layout --eitype-xkb-variant --pre-output-command --post-output-command --pre-recording-command --wait-for-modifier-release --no-wait-for-modifier-release --modifier-release-timeout-ms --spoken-punctuation --shift-enter-newlines --no-shift-enter-newlines --smart-auto-submit --no-smart-auto-submit --filter-fillers --no-filter-fillers --append-text --vad --vad-threshold --vad-backend --vad-min-speech-ms --help --version daemon transcribe transcribe-worker setup config info configure status record meeting check-update help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --config)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -c)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --engine)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --language)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --threads)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --gpu-device)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --secondary-model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --initial-prompt)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --whisper-mode)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --remote-endpoint)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --remote-model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --remote-api-key)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --soniox-api-key)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --hotkey)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cancel-key)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --model-modifier)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --audio-device)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --max-duration)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --duck-media-volume)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --duck-media-fade-ms)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --restore-clipboard-delay-ms)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --driver)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --paste-keys)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --file-path)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --file-mode)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --pre-type-delay)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --wtype-delay)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --type-delay)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dotool-xkb-layout)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --dotool-xkb-variant)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --eitype-xkb-layout)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --eitype-xkb-variant)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --pre-output-command)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --post-output-command)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --pre-recording-command)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --modifier-release-timeout-ms)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --append-text)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --vad-threshold)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --vad-backend)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --vad-min-speech-ms)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__check__subcmd__update)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config)
            opts="-h --help set unset get schema help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__get)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__help)
            opts="set unset get schema help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__help__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__help__subcmd__schema)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__help__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__help__subcmd__unset)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__schema)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__set)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__config__subcmd__unset)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__configure)
            opts="-h --force-package-mode --probe-audio-devices --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__daemon)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help)
            opts="daemon transcribe transcribe-worker setup config info configure status record meeting check-update help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__check__subcmd__update)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__config)
            opts="set unset get schema"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__config__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__config__subcmd__schema)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__config__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__config__subcmd__unset)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__configure)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__daemon)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__info)
            opts="variants devices models accel engines styles"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__info__subcmd__accel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__info__subcmd__devices)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__info__subcmd__engines)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__info__subcmd__models)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__info__subcmd__styles)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__info__subcmd__variants)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting)
            opts="start stop pause resume status list export show delete label summarize"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__export)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__label)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__resume)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__meeting__subcmd__summarize)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__record)
            opts="start stop toggle cancel"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__record__subcmd__cancel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__record__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__record__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__record__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup)
            opts="check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__check)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__compositor)
            opts="hyprland sway river"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__hyprland)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__river)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__compositor__subcmd__sway)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__dms)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__gpu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__model)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__npu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__onnx)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__parakeet)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__quickshell)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__systemd)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__vad)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__variant)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__setup__subcmd__waybar)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__transcribe)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__help__subcmd__transcribe__subcmd__worker)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info)
            opts="-h --help variants devices models accel engines styles help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__accel)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__devices)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__engines)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help)
            opts="variants devices models accel engines styles help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help__subcmd__accel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help__subcmd__devices)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help__subcmd__engines)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help__subcmd__models)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help__subcmd__styles)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__help__subcmd__variants)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__models)
            opts="-h --json --engine --verify --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --engine)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__styles)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__info__subcmd__variants)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting)
            opts="-h --help start stop pause resume status list export show delete label summarize help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__delete)
            opts="-f -h --force --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__export)
            opts="-f -o -h --format --output --timestamps --speakers --metadata --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -f)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help)
            opts="start stop pause resume status list export show delete label summarize help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__delete)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__export)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__label)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__pause)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__resume)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__help__subcmd__summarize)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__label)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__list)
            opts="-l -h --limit --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -l)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__pause)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__resume)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__show)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__start)
            opts="-t -h --title --diarization --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --title)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -t)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --diarization)
                    COMPREPLY=($(compgen -W "simple ml" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__status)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__stop)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__meeting__subcmd__summarize)
            opts="-f -o -h --format --output --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -f)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record)
            opts="-h --help start stop toggle cancel help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__cancel)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__help)
            opts="start stop toggle cancel help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__help__subcmd__cancel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__help__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__help__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__help__subcmd__toggle)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__start)
            opts="-h --type --clipboard --paste --file --model --profile --auto-submit --no-auto-submit --shift-enter-newlines --no-shift-enter-newlines --no-osd --smart-auto-submit --no-smart-auto-submit --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --profile)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__stop)
            opts="-h --type --clipboard --paste --wait --json --timeout --wait-file --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --timeout)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --wait-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__record__subcmd__toggle)
            opts="-h --type --clipboard --paste --file --model --profile --auto-submit --no-auto-submit --shift-enter-newlines --no-shift-enter-newlines --no-osd --smart-auto-submit --no-smart-auto-submit --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --profile)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup)
            opts="-h --download --model --quiet --no-post-install --activate --progress-format --help check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --progress-format)
                    COMPREPLY=($(compgen -W "human json" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__check)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor)
            opts="-h --help hyprland sway river help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__help)
            opts="hyprland sway river help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__hyprland)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__river)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__help__subcmd__sway)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__hyprland)
            opts="-h --uninstall --status --show --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__river)
            opts="-h --uninstall --status --show --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__compositor__subcmd__sway)
            opts="-h --uninstall --status --show --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__dms)
            opts="-h --install --uninstall --qml --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__gpu)
            opts="-h --enable --disable --status --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help)
            opts="check systemd waybar dms model gpu npu variant onnx parakeet compositor vad quickshell help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__check)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__compositor)
            opts="hyprland sway river"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__hyprland)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__river)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__compositor__subcmd__sway)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__dms)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__gpu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__model)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__npu)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__onnx)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__parakeet)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__quickshell)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__systemd)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__vad)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__variant)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__help__subcmd__waybar)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__model)
            opts="-h --list --set --restart --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --set)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__npu)
            opts="-h --enable --disable --status --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__onnx)
            opts="-h --enable --disable --status --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__parakeet)
            opts="-h --enable --disable --status --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__quickshell)
            opts="-h --target --source --force --print-bindings --bridge --bridge-target --skip-bridge --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --source)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --bridge)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --bridge-target)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__systemd)
            opts="-h --uninstall --status --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__vad)
            opts="-h --status --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__variant)
            opts="-h --to --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__setup__subcmd__waybar)
            opts="-h --json --css --install --uninstall --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__status)
            opts="-h --follow --format --extended --icon-theme --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --format)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --icon-theme)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__transcribe)
            opts="-h --engine --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --engine)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        voxtype__subcmd__transcribe__subcmd__worker)
            opts="-h --model --language --translate --threads --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --language)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --threads)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _voxtype -o nosort -o bashdefault -o default voxtype
else
    complete -F _voxtype -o bashdefault -o default voxtype
fi

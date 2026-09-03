# Fish completion for voxtype

# Disable file completion by default
complete -c voxtype -f

# Global options
complete -c voxtype -s c -l config -d 'Path to config file' -r -F
complete -c voxtype -s v -l verbose -d 'Increase verbosity'
complete -c voxtype -s q -l quiet -d 'Quiet mode (errors only)'
complete -c voxtype -l clipboard -d 'Force clipboard mode'
complete -c voxtype -l model -d 'Override whisper model' -xa 'tiny tiny.en base base.en small small.en medium medium.en large-v3'
complete -c voxtype -l hotkey -d 'Override hotkey' -xa 'SCROLLLOCK PAUSE RIGHTALT F13 F14 F15 F16 F17 F18 F19 F20'
complete -c voxtype -l hotkey-backend -d 'Override Linux hotkey backend' -xa 'evdev portal auto'
complete -c voxtype -s h -l help -d 'Print help'
complete -c voxtype -s V -l version -d 'Print version'

# Commands
complete -c voxtype -n __fish_use_subcommand -a daemon -d 'Run as background daemon'
complete -c voxtype -n __fish_use_subcommand -a transcribe -d 'Transcribe an audio file'
complete -c voxtype -n __fish_use_subcommand -a setup -d 'Check dependencies and download models'
complete -c voxtype -n __fish_use_subcommand -a config -d 'Show current configuration'
complete -c voxtype -n __fish_use_subcommand -a help -d 'Print help information'

# Transcribe subcommand
complete -c voxtype -n '__fish_seen_subcommand_from transcribe' -s o -l output -d 'Output file' -r -F
complete -c voxtype -n '__fish_seen_subcommand_from transcribe' -l model -d 'Override whisper model' -xa 'tiny tiny.en base base.en small small.en medium medium.en large-v3'
complete -c voxtype -n '__fish_seen_subcommand_from transcribe' -F

# Setup subcommand
complete -c voxtype -n '__fish_seen_subcommand_from setup' -l download -d 'Download whisper model'
complete -c voxtype -n '__fish_seen_subcommand_from setup' -l model -d 'Model to download' -xa 'tiny tiny.en base base.en small small.en medium medium.en large-v3'

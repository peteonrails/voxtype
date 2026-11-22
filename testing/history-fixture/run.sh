#!/usr/bin/env bash
# Run the history acceptance scripts (testing/test-history-*.py) in a local
# container that has the layout they assert. See the Dockerfile for why.
#
#   testing/history-fixture/run.sh                    every script, every case
#   testing/history-fixture/run.sh gtk-stale          one script, every case
#   testing/history-fixture/run.sh gtk-stale error    one script, one case
#
# The first argument is a script name, with or without the "test-history-"
# prefix and ".py". The second is a case label as printed in results.tsv.
#
# Each case runs in its own container: fresh /work/home and /work/scratch,
# no network, the source tree mounted read-only, and no host display, D-Bus,
# audio or input devices. Results go to $OUT:
#
#   results.tsv              script, case, PASS/FAIL, seconds
#   <script>__<case>.log     the script's stdout and stderr
#   <script>__<case>/        what it wrote to /work/out (screenshots, JSON)
#
# Environment:
#   DOCKER        docker command                 (default: sudo docker)
#   OUT           results directory              (default: under ~/.cache)
#   SKIP_BUILD    set to reuse the image and binaries from an earlier run
#   CASE_TIMEOUT  seconds before a case is killed (default: 300)
set -uo pipefail

here=$(cd "$(dirname "$0")" && pwd)
src=$(cd "$here/../.." && pwd)
read -ra docker <<<"${DOCKER:-sudo docker}"
OUT=${OUT:-${XDG_CACHE_HOME:-$HOME/.cache}/voxtype-131/e2e/run-$(date +%Y%m%d-%H%M%S)}
CASE_TIMEOUT=${CASE_TIMEOUT:-300}
IMAGE=voxtype-history-fixture
# CI's feature set, plus the feature that gates voxtype-meetings-gtk4.
FEATURES=gguf,cohere-gguf,parakeet-gguf,meetings-gtk4

only_script=${1:-}
only_script=${only_script##*/}
only_script=${only_script#test-history-}
only_script=${only_script%.py}
only_case=${2:-}

# script: testing/test-history-<script>.py
# needs:  none  the script must see no display or session bus (the ones that
#               want a compositor start their own headless Sway)
#         dbus  a private session bus
#         xvfb  a private Xvfb display and session bus
# fixed:  environment that selects a variant, comma-separated, or -
# var:    the environment variable that selects the case, or -
# cases:  every value the script accepts for it
#
# script                   needs fixed                                 var                                cases
table='
configurator               none  -                                     VOX_HISTORY_CONFIG_CASE            explicit default
file-replay                none  -                                     -                                  -
gtk                        none  -                                     -                                  -
gtk-controls               xvfb  -                                     VOX_HISTORY_GTK_CASE               controls search details correction delete copy paste
gtk-controls               xvfb  VOX_HISTORY_GTK_LAUNCH=cli            VOX_HISTORY_GTK_CASE               controls search details correction delete copy paste
gtk-forwarding             xvfb  -                                     VOX_HISTORY_GTK_FORWARD_PATH       absolute same-relative different-relative direct-same-relative direct-different-relative default-relative missing-parent-relative
gtk-meeting-link           xvfb  -                                     -                                  -
gtk-responsive             xvfb  -                                     VOX_HISTORY_GTK_RESPONSIVENESS     replay search delete
gtk-stale                  xvfb  -                                     VOX_HISTORY_GTK_STALE              success error
guidance                   none  -                                     -                                  -
launch                     none  -                                     VOX_HISTORY_LAUNCH_CASE            default
launch                     dbus  -                                     VOX_HISTORY_LAUNCH_CASE            forwarded-config meeting-link
launch                     dbus  VOX_HISTORY_LAUNCH_CASE=meeting-link  VOX_HISTORY_LINK_FAILURE_CASE      search-reopen cross-context copy-search
native-tray                dbus  -                                     VOX_HISTORY_TRAY_CASE              explicit legacy
native-tray-default        dbus  -                                     VOX_HISTORY_TRAY_CASE              default
native-tray-convergence    dbus  -                                     VOX_HISTORY_TRAY_CASE              default
omarchy-launch             none  -                                     VOX_HISTORY_OMARCHY_CASE           panel types async-control panel-sync
omarchy-launch             dbus  -                                     VOX_HISTORY_OMARCHY_CASE           meeting-link
omarchy-launch             dbus  VOX_HISTORY_OMARCHY_CASE=meeting-link VOX_HISTORY_LINK_FAILURE_CASE      search-reopen cross-context copy-search
privacy-advice             none  -                                     -                                  -
private-save               none  -                                     -                                  -
qml                        none  VOX_HISTORY_QML_SURFACE=plugin        VOX_HISTORY_QML_CASE               list search controls details correction delete copy paste raw-fallback
qml                        none  VOX_HISTORY_QML_SURFACE=standalone    VOX_HISTORY_QML_CASE               list search controls details correction delete copy paste raw-fallback
quickshell                 none  -                                     VOX_HISTORY_QS_CASE                transport window
settings-external          none  VOX_HISTORY_SETTINGS_CASE=explicit    VOX_HISTORY_SETTINGS_EXTERNAL_CASE profile history pending-history replacements scalar
settings-external          none  VOX_HISTORY_SETTINGS_CASE=default     VOX_HISTORY_SETTINGS_EXTERNAL_CASE profile history pending-history replacements scalar
settings-graphical         none  -                                     VOX_HISTORY_SETTINGS_CASE          explicit default
tui                        none  -                                     VOX_HISTORY_SMOKE_CASE             reader search correct correct-leading-dash delete controls paste copy copy-raw-fallback copy-corrected copy-corrected-empty
tui                        xvfb  -                                     VOX_HISTORY_SMOKE_CASE             meeting-nav
'

# The environment the scripts were written against. Nothing is inherited.
base_env=(
  HOME=/work/home PATH=/usr/local/bin:/usr/bin:/bin LC_ALL=C.UTF-8
  TMPDIR=/work/scratch XDG_RUNTIME_DIR=/work/runtime
  XDG_CONFIG_HOME=/work/home/config XDG_DATA_HOME=/work/home/data
  PYTHONDONTWRITEBYTECODE=1
  VOXTYPE_HISTORY_TEST_BIN=/work/target/debug/voxtype
  VOXTYPE_HISTORY_TEST_GTK_BIN=/work/target/debug/voxtype-meetings-gtk4
)

[ ! -e "$OUT/results.tsv" ] || { echo "$OUT already holds a run; choose another OUT"; exit 1; }
mkdir -p "$OUT"
echo "Results: $OUT"

# On any exit: remove the container of an interrupted case, and hand the
# results back (the scripts write as uid 65532 with a 077 umask).
box=
cleanup() {
  [ -z "$box" ] || "${docker[@]}" rm -f "$box" >/dev/null
  "${docker[@]}" run --rm --user 0:0 -v "$OUT":/o "$IMAGE" chown -R "$(id -u):$(id -g)" /o
}
trap cleanup EXIT
trap 'exit 130' INT TERM

if [ -z "${SKIP_BUILD:-}" ]; then
  echo "Building image (log: $OUT/image-build.log)"
  "${docker[@]}" build -t "$IMAGE" "$here" >"$OUT/image-build.log" 2>&1 || { echo "Image build failed"; exit 1; }
  echo "Building voxtype and voxtype-meetings-gtk4 (log: $OUT/cargo-build.log)"
  "${docker[@]}" run --rm --cap-drop=ALL --security-opt=no-new-privileges \
    -v "$src":/src:ro \
    -v voxtype-history-target:/work/target -v voxtype-history-cargo:/work/cargo \
    -e CARGO_HOME=/work/cargo -e CARGO_TARGET_DIR=/work/target \
    "$IMAGE" cargo build --locked --features "$FEATURES" --bin voxtype --bin voxtype-meetings-gtk4 \
    >"$OUT/cargo-build.log" 2>&1 || { echo "cargo build failed"; exit 1; }
fi

run_case() {
  local script=$1 needs=$2 label=$3; shift 3   # the rest: VAR=value ...
  local name="$script${label:+__$label}"
  local wrap=()
  case $needs in
    none) ;;
    dbus) wrap=(dbus-run-session --) ;;
    xvfb) wrap=(xvfb-run -a -s '-screen 0 1280x1000x24 -nolisten tcp' dbus-run-session --) ;;
    *) echo "Unknown needs '$needs' for $script"; exit 1 ;;
  esac
  mkdir -p "$OUT/$name" && chmod 777 "$OUT/$name"

  # The container idles and the script runs through exec, as it did where
  # these scripts were written. Orphaned children are not reaped there.
  local status=PASS start=$SECONDS
  box=$("${docker[@]}" run -d --network=none --cap-drop=ALL --security-opt=no-new-privileges \
    --pids-limit=512 --memory=12g \
    -v "$src":/src:ro -v voxtype-history-target:/work/target:ro -v "$OUT/$name":/work/out \
    "$IMAGE" sleep infinity) || { echo "Could not start a container"; exit 1; }
  timeout "$CASE_TIMEOUT" "${docker[@]}" exec "$box" env -i "${base_env[@]}" "$@" \
    "${wrap[@]}" python3 "testing/test-history-$script.py" >"$OUT/$name.log" 2>&1
  local rc=$?
  [ $rc -eq 0 ] || status=FAIL
  [ $rc -ne 124 ] || echo "run.sh: killed after ${CASE_TIMEOUT}s" >>"$OUT/$name.log"
  "${docker[@]}" rm -f "$box" >/dev/null
  box=

  printf '%s\t%s\t%s\t%s\n' "test-history-$script.py" "${label:--}" "$status" "$((SECONDS - start))" \
    | tee -a "$OUT/results.tsv"
}

while read -r script needs fixed var cases; do
  [ -n "$script" ] || continue
  [ -z "$only_script" ] || [ "$script" = "$only_script" ] || continue
  [ "$fixed" != - ] || fixed=
  for case in $cases; do
    assignments=${fixed//,/ }
    [ "$var" = - ] || assignments+=" $var=$case"
    # Label: the values, e.g. "cli+paste" or "meeting-link+copy-search".
    label=
    for assignment in $assignments; do label+="${label:++}${assignment#*=}"; done
    [ -z "$only_case" ] || [ "$label" = "$only_case" ] || continue
    # shellcheck disable=SC2086  # assignments are split on purpose
    run_case "$script" "$needs" "$label" $assignments </dev/null
  done
done <<<"$table"

[ -f "$OUT/results.tsv" ] || { echo "Nothing matched '$*'"; exit 1; }
echo "$(grep -c PASS$'\t' "$OUT/results.tsv") passed, $(grep -c FAIL$'\t' "$OUT/results.tsv") failed. Results: $OUT/results.tsv"
! grep -q FAIL$'\t' "$OUT/results.tsv"

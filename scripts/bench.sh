#!/usr/bin/env bash
# Reproducible multi-terminal speed and latency suite for Koral M2 Task 4.
# Requires a live Wayland/X11 session; generated artifacts stay in a temp dir.
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/bench.sh --terminals koral,kitty,ghostty,kwi --runs 5 --json /tmp/bench.json [options]

Options:
  --terminals LIST   Comma-separated terminals to measure (koral,kitty,ghostty,kwi)
  --runs N           Measurements per terminal (positive integer)
  --json PATH        Write the JSON report here
  --config MODE      Profile mode: NONE (default, isolated empty XDG dirs) or user
  --cat-mb N         Payload size in MB (default 100)
  --timeout S        Per-launch timeout in seconds (default 180)
  --no-latency       Skip input-to-photon measurement
  --allow-real-desktop
                     Allow the real desktop session (unsafe; deliberate use only)
  -h, --help         Show this help

Invalid arguments exit 2. Missing terminals are skipped with a JSON reason.
EOF
}

die() { printf 'bench.sh: %s\n' "$*" >&2; exit 2; }

terminals=""
runs=""
json_out=""
config_mode="NONE"
cat_mb=100
timeout_s=180
measure_latency=1
allow_real_desktop=0

while (($#)); do
    case "$1" in
        --terminals) (($# >= 2)) || die "--terminals requires a value"; terminals=$2; shift 2 ;;
        --runs) (($# >= 2)) || die "--runs requires a value"; runs=$2; shift 2 ;;
        --json) (($# >= 2)) || die "--json requires a value"; json_out=$2; shift 2 ;;
        --config) (($# >= 2)) || die "--config requires a value"; config_mode=$2; shift 2 ;;
        --cat-mb) (($# >= 2)) || die "--cat-mb requires a value"; cat_mb=$2; shift 2 ;;
        --timeout) (($# >= 2)) || die "--timeout requires a value"; timeout_s=$2; shift 2 ;;
        --no-latency) measure_latency=0; shift ;;
        --allow-real-desktop) allow_real_desktop=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1" ;;
    esac
done

if (( ! allow_real_desktop )); then
    wayland_target=${WAYLAND_DISPLAY:-}
    x11_target=${DISPLAY:-}
    real_desktop=0
    if [[ -n "$wayland_target" && "$wayland_target" != wayland-koral-* ]]; then
        real_desktop=1
    fi
    if [[ "$x11_target" == :0 || "$x11_target" == :1 ]]; then
        real_desktop=1
    fi
    if (( real_desktop )); then
        printf 'bench.sh: refusing to run on the real desktop session\n' >&2
        printf 'bench.sh: WAYLAND_DISPLAY=%s DISPLAY=%s\n' "${WAYLAND_DISPLAY:-<unset>}" "${DISPLAY:-<unset>}" >&2
        printf 'bench.sh: use an isolated virtual monitor (e.g. WAYLAND_DISPLAY=wayland-koral-<name>), or pass --allow-real-desktop deliberately.\n' >&2
        exit 2
    fi
fi

[[ -n "$terminals" ]] || die "--terminals is required"
[[ -n "$runs" && "$runs" =~ ^[1-9][0-9]*$ ]] || die "--runs must be a positive integer"
[[ -n "$json_out" ]] || die "--json is required"
[[ "$config_mode" == NONE || "$config_mode" == user ]] || die "--config must be NONE or user"
[[ "$cat_mb" =~ ^[1-9][0-9]*$ ]] || die "--cat-mb must be a positive integer"
[[ "$timeout_s" =~ ^[1-9][0-9]*$ ]] || die "--timeout must be a positive integer"

valid_terminals=",koral,kitty,ghostty,kwi,alacritty,"
IFS=',' read -r -a terminal_list <<< "$terminals"
for terminal in "${terminal_list[@]}"; do
    [[ -n "$terminal" && "$valid_terminals" == *",$terminal,"* ]] || die "invalid terminal: $terminal"
done

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
bench_dir=$(mktemp -d "${TMPDIR:-/tmp}/koral-bench.XXXXXXXX")
trap 'rm -rf "$bench_dir"' EXIT

now_ns() { date +%s%N; }
json_escape() {
    local value=${1//\\/\\\\}
    value=${value//\"/\\\"}
    value=${value//$'\n'/\\n}
    value=${value//$'\t'/\\t}
    printf '%s' "$value"
}
median() {
    printf '%s\n' "$@" | sort -n | awk '{
        a[NR] = $1
    }
    END {
        if (NR == 0) { printf "null"; exit }
        mid = int((NR + 1) / 2)
        if (NR % 2 == 1) printf "%.3f", a[mid]
        else printf "%.3f", (a[mid - 1] + a[mid]) / 2
    }'
}
median_or_null() {
    if (($#)); then median "$@"; else printf 'null'; fi
}
log() { printf '[bench] %s\n' "$*" >&2; }

# Reuse the same wrapped base64 payload for every terminal and run.
payload_bytes=$((cat_mb * 1000000))
random_bytes=$((payload_bytes * 3 / 4))

generate_payload() {
    local target=$1
    head -c "$random_bytes" /dev/urandom > "$target.raw"
    base64 "$target.raw" > "$target.full"
    head -c "$payload_bytes" "$target.full" > "$target"
    rm -f "$target.raw" "$target.full"
}

cat > "$bench_dir/shell.tmpl" <<'SHELL'
#!/usr/bin/env bash
set -u
BENCH_DIR=${BENCH_DIR:?}
now() { date +%s%N; }
printf '%s' "$(now)" > "$BENCH_DIR/started"
cat "$BENCH_DIR/data"
printf '%s' "$(now)" > "$BENCH_DIR/ended"
# VmHWM from the terminal process that spawned this shell (walk up to a known terminal).
pid=$$
for _ in 1 2 3 4 5 6 7 8; do
    pid=$(awk '/^PPid:/ {print $2}' "/proc/$pid/status" 2>/dev/null) || break
    [[ -n "$pid" && "$pid" != 0 ]] || break
    cmdline=$(tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null || true)
    exe=$(readlink -f "/proc/$pid/exe" 2>/dev/null || true)
    if [[ "$cmdline" == *kitty* || "$cmdline" == *koral* || "$cmdline" == *ghostty* || "$cmdline" == *kwi* || "$cmdline" == *alacritty* ]] || \
       case "${exe##*/}" in koral|kitty|ghostty|kwi|alacritty) true ;; *) false ;; esac; then
        awk '/^VmHWM:/ {print $2}' "/proc/$pid/status" > "$BENCH_DIR/rss" 2>/dev/null || true
        break
    fi
done
# Colored TUI throughput: emit a fixed burst of full-width colored frames.
printf '\033[H\033[2J'
printf '%s' "$(now)" > "$BENCH_DIR/tui_started"
frame=0
for _ in $(seq 1 60); do
    printf '\033[H\033[2J\033[48;2;%d;%d;%dm' $((frame % 256)) $(((frame * 7) % 256)) $(((frame * 13) % 256))
    for row in $(seq 1 24); do
        printf '%*s\n' 80 ' '
    done
    frame=$((frame + 1))
    printf '%s' "$frame" > "$BENCH_DIR/frames"
done
printf '%s' "$(now)" > "$BENCH_DIR/tui_ended"
SHELL

binary_for() {
    case "$1" in
        koral) printf '%s' "${KORAL_BIN:-$repo_root/target/release/koral}" ;;
        kitty) command -v kitty || true ;;
        ghostty) command -v ghostty || true ;;
        kwi) command -v kwi || true ;;
        alacritty) command -v alacritty || true ;;
    esac
}

absolute_binary() {
    local binary=$1
    [[ -n "$binary" && -x "$binary" ]] || return 1
    if [[ "$binary" == */* ]]; then
        (cd -- "$(dirname -- "$binary")" && printf '%s/%s\n' "$PWD" "$(basename -- "$binary")")
    else
        command -v -- "$binary"
    fi
}
version_for() {
    local binary=$1
    [[ -n "$binary" && -x "$binary" ]] || { printf 'null'; return; }
    local output
    output=$(timeout 10 "$binary" --version 2>&1 || true)
    printf '"%s"' "$(json_escape "$(printf '%s' "$output" | head -n 1)")"
}
config_args_for() {
    local terminal=$1
    [[ "$config_mode" == NONE ]] || { printf ''; return; }
    case "$terminal" in
        kitty) printf '%s' '--config NONE' ;;
        # Ghostty and kwi have no --config NONE flag; an empty XDG_CONFIG_HOME
        # gives them a clean profile without user config files.
        ghostty|kwi|alacritty) printf '' ;;
        *) printf '' ;;
    esac
}

session_type=${XDG_SESSION_TYPE:-unknown}
compositor="unknown"
if [[ "$session_type" == wayland ]]; then
    if command -v hyprctl >/dev/null 2>&1 && hyprctl version >/dev/null 2>&1; then
        compositor="Hyprland $(hyprctl version | head -n 1 | awk '{print $2}')"
    elif [[ -n "${SWAY_CURRENT_DIR:-}" ]] || command -v swaymsg >/dev/null 2>&1; then
        compositor="sway"
    else
        compositor="Wayland (unidentified)"
    fi
elif [[ "$session_type" == x11 ]]; then
    compositor="X11 (unidentified)"
fi

system_json() {
    local kernel gpu
    kernel=$(uname -sr)
    gpu=$(lspci 2>/dev/null | grep -iE 'vga|3d controller' | head -n 1 | cut -d: -f3- | sed 's/^ *//') || gpu="unknown"
    printf '{"kernel":"%s","session_type":"%s","compositor":"%s","wayland_display":"%s","display":"%s","gpu":"%s"}' \
        "$(json_escape "$kernel")" "$(json_escape "$session_type")" "$(json_escape "$compositor")" \
        "$(json_escape "${WAYLAND_DISPLAY:-}")" "$(json_escape "${DISPLAY:-}")" "$(json_escape "$gpu")"
}

terminal_json() {
    local name=$1 binary=$2 version_json=$3 status=$4 reason=$5
    shift 5
    local runs_json="$*"
    printf '{"name":"%s","binary":"%s","version":%s,"config_mode":"%s","status":"%s","reason":"%s","runs":[%s],"summary":%s}' \
        "$name" "$(json_escape "$binary")" "$version_json" "$config_mode" "$status" \
        "$(json_escape "$reason")" "$runs_json" "$(summarize_runs "$runs_json")"
}

summarize_runs() {
    python3 -c '
import json,sys
raw=sys.argv[1]
runs=json.loads("["+raw+"]") if raw.strip() else []
def med(key, sub=None):
    vals=[]
    for r in runs:
        v=r.get(key)
        if sub:
            v=(v or {}).get(sub)
        if isinstance(v,(int,float)): vals.append(float(v))
    if not vals: return None
    vals.sort(); n=len(vals)
    return round(vals[n//2] if n%2 else (vals[n//2-1]+vals[n//2])/2,3)
print(json.dumps({"launch_ms":med("launch_ms"),"cat_ms":med("cat_ms"),"peak_rss_kib":med("peak_rss_kib"),"tui_fps":med("tui_fps"),"latency_p50_ms":med("latency","p50_ms"),"latency_p99_ms":med("latency","p99_ms")}))' "$1"
}

run_one() {
    local terminal=$1 binary=$2 config_args=$3 index=$4
    local run_dir="$bench_dir/${terminal}-${index}"
    mkdir -p "$run_dir"
    local -a env_base=(-u KORAL_SHELL -u KORAL_SOCKET -u KORAL_FRAME_LOG -u KORAL_GRAPHICS_LOG
        BENCH_DIR="$run_dir" TMPDIR="$run_dir")
    if [[ "$config_mode" == NONE ]]; then
        mkdir -p "$run_dir"/{home,config,data,state,cache}
        env_base+=(HOME="$run_dir/home" XDG_CONFIG_HOME="$run_dir/config"
            XDG_DATA_HOME="$run_dir/data" XDG_STATE_HOME="$run_dir/state"
            XDG_CACHE_HOME="$run_dir/cache")
    fi
    # Keep the isolated compositor connection in XDG_RUNTIME_DIR intact.
    if [[ "$terminal" == koral ]]; then
        env_base+=(KORAL_SOCKET="$run_dir/koral.sock")
    fi
    ln -s "$bench_dir/payload" "$run_dir/payload"
    cp "$bench_dir/shell.tmpl" "$run_dir/shell"
    chmod +x "$run_dir/shell"

    local launched_ns started_ns ended_ns tui_started_ns tui_ended_ns rss_kib="null" frames=0
    local -a launch_args=()
    read -r -a launch_args <<< "$config_args"
    case "$terminal" in
        koral|alacritty|ghostty) launch_args+=(-e) ;;
    esac
    launched_ns=$(now_ns)
    timeout --signal=TERM --kill-after=5s "$timeout_s" \
        env "${env_base[@]}" "$binary" "${launch_args[@]}" "$run_dir/shell" \
        >/dev/null 2>"$run_dir/launch.err" </dev/null &
    local terminal_pid=$!
    local deadline=$(( $(date +%s) + timeout_s ))
    while [[ ! -s "$run_dir/tui_ended" ]] && kill -0 "$terminal_pid" 2>/dev/null && (( $(date +%s) < deadline )); do
        [[ -f "$run_dir/frames" ]] && frames=$(cat "$run_dir/frames")
        sleep 0.01
    done
    kill "$terminal_pid" 2>/dev/null || true
    wait "$terminal_pid" 2>/dev/null || true

    if [[ ! -s "$run_dir/started" || ! -s "$run_dir/ended" || ! -s "$run_dir/tui_started" || ! -s "$run_dir/tui_ended" ]]; then
        if [[ -s "$run_dir/launch.err" ]]; then
            log "$terminal launch failed: $(head -c 2048 "$run_dir/launch.err")"
        fi
        printf '%s' '{"status":"unavailable","reason":"benchmark process exited or timed out before completing","launch_ms":null,"cat_ms":null,"peak_rss_kib":null,"tui_fps":null,"latency":{"status":"unavailable","samples_ms":[]}}'
        return
    fi
    [[ -s "$run_dir/frames" ]] && frames=$(cat "$run_dir/frames")
    [[ -s "$run_dir/started" ]] && started_ns=$(cat "$run_dir/started")
    [[ -s "$run_dir/ended" ]] && ended_ns=$(cat "$run_dir/ended")
    [[ -s "$run_dir/tui_started" ]] && tui_started_ns=$(cat "$run_dir/tui_started")
    [[ -s "$run_dir/tui_ended" ]] && tui_ended_ns=$(cat "$run_dir/tui_ended")
    [[ -s "$run_dir/rss" ]] && rss_kib=$(cat "$run_dir/rss")

    local launch_ms=0 cat_ms=0 tui_fps=0
    [[ -n "${started_ns:-}" && -n "${launched_ns:-}" ]] && launch_ms=$(( (started_ns - launched_ns) / 1000000 ))
    [[ -n "${ended_ns:-}" && -n "${started_ns:-}" ]] && cat_ms=$(( (ended_ns - started_ns) / 1000000 ))
    if [[ -n "${tui_ended_ns:-}" && -n "${tui_started_ns:-}" ]] && (( tui_ended_ns > tui_started_ns )); then
        tui_fps=$(awk -v f="$frames" -v s="$tui_started_ns" -v e="$tui_ended_ns" 'BEGIN {printf "%.2f", f*1000000000/(e-s)}')
    fi

    local latency_json='{"status":"skipped","reason":"disabled","samples_ms":[],"p50_ms":null,"p95_ms":null,"p99_ms":null}'
    if (( measure_latency )) && [[ -f "$script_dir/bench-latency.py" ]]; then
        local lat_dir="$run_dir/lat"
        mkdir -p "$lat_dir"
        local lat_args=$config_args
        case "$terminal" in
            alacritty) lat_args="$config_args --title koral-bench-$terminal-$index" ;;
            kitty|koral|ghostty|kwi) : ;;
        esac
        timeout --signal=TERM --kill-after=5s "$timeout_s" \
            env "${env_base[@]}" python3 "$script_dir/bench-latency.py" \
                --terminal "$terminal" --binary "$binary" --run-dir "$lat_dir" --runs 1 \
                --config="$lat_args" --json "$run_dir/latency.json" >/dev/null 2>"$run_dir/latency.err" || true
        if [[ -s "$run_dir/latency.json" ]]; then
            latency_json=$(python3 -c '
import json,sys
d=json.load(open(sys.argv[1]))
keys=("terminal","binary","version","methodology","samples_ms","key_events","p50_ms","p95_ms","p99_ms","status","reason")
d={k:d.get(k) for k in keys}
print(json.dumps(d))' "$run_dir/latency.json")
        else
            latency_json='{"status":"unavailable","reason":"latency helper produced no output","samples_ms":[],"p50_ms":null,"p95_ms":null,"p99_ms":null}'
        fi
    fi

    printf '{"status":"ok","launch_ms":%d,"cat_ms":%d,"peak_rss_kib":%s,"tui_fps":%s,"latency":%s}' \
        "$launch_ms" "$cat_ms" "$rss_kib" "$tui_fps" "$latency_json"
}

report="[]"
for terminal in "${terminal_list[@]}"; do
    binary=$(absolute_binary "$(binary_for "$terminal")" 2>/dev/null || true)
    version_json=$(version_for "$binary")
    if [[ -z "$binary" || ! -x "$binary" ]]; then
        log "skipping $terminal (not installed)"
        report=$(python3 -c 'import json,sys; a=json.loads(sys.argv[1]); a.append(json.loads(sys.argv[2])); print(json.dumps(a))' \
            "$report" "$(terminal_json "$terminal" "$binary" "null" "skipped" "not installed" "")")
        continue
    fi
    [[ -f "$bench_dir/payload" ]] || generate_payload "$bench_dir/payload"
    config_args=$(config_args_for "$terminal")
    log "measuring $terminal ($version_json), $runs run(s)"
    runs_json=""
    for ((i=1; i<=runs; i++)); do
        run_json=$(run_one "$terminal" "$binary" "$config_args" "$i")
        [[ -z "$runs_json" ]] || runs_json+=","
        runs_json+="$run_json"
    done
    terminal_status=ok
    terminal_reason=""
    if [[ "$runs_json" == *'"status":"unavailable","reason":"benchmark process'* ]]; then
        terminal_status=unavailable
        terminal_reason="one or more benchmark runs did not complete"
    fi
    report=$(python3 -c 'import json,sys; a=json.loads(sys.argv[1]); a.append(json.loads(sys.argv[2])); print(json.dumps(a))' \
        "$report" "$(terminal_json "$terminal" "$binary" "$version_json" "$terminal_status" "$terminal_reason" "$runs_json")")
done

generated_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
final_json=$(python3 -c '
import json,sys
report={"schema_version":1,"generated_at":sys.argv[1],"environment":{"system":json.loads(sys.argv[2]),"payload_bytes":int(sys.argv[3]),"runs":int(sys.argv[4]),"config_mode":sys.argv[5]},"terminals":json.loads(sys.argv[6])}
print(json.dumps(report,indent=2,sort_keys=True))' \
    "$generated_at" "$(system_json)" "$payload_bytes" "$runs" "$config_mode" "$report")
printf '%s\n' "$final_json" > "$bench_dir/bench.json"
mkdir -p "$(dirname -- "$json_out")"
cp "$bench_dir/bench.json" "$json_out.tmp.$$"
mv "$json_out.tmp.$$" "$json_out"
log "report written to $json_out"
printf '%s\n' "$final_json"

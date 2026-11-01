#!/bin/bash
# Usage: scripts/bench.sh <koral|kitty>, from a Wayland session and the repository root.
# Prints the launch-to-shell time, the time to cat 100 MB and the terminal's peak RSS.
set -eu
terminal=$1
export BENCH_DIR
BENCH_DIR=$(mktemp -d)
trap 'rm -rf "$BENCH_DIR"' EXIT

head -c 75000000 /dev/urandom | base64 > "$BENCH_DIR/data"
cat > "$BENCH_DIR/shell" <<'S'
#!/bin/bash
date +%s%N > "$BENCH_DIR/started"
cat "$BENCH_DIR/data"
date +%s%N > "$BENCH_DIR/ended"
awk '/VmHWM/ { print $2 }' "/proc/$PPID/status" > "$BENCH_DIR/rss"
S
chmod +x "$BENCH_DIR/shell"

launched=$(date +%s%N)
case $terminal in
    koral) KORAL_SHELL=$BENCH_DIR/shell ./target/release/koral ;;
    kitty) kitty --config NONE "$BENCH_DIR/shell" ;;
    *) echo "usage: $0 <koral|kitty>" >&2; exit 2 ;;
esac

started=$(cat "$BENCH_DIR/started")
ended=$(cat "$BENCH_DIR/ended")
printf '%s: start %d ms, cat %d ms, peak %d MiB\n' "$terminal" \
    $(((started - launched) / 1000000)) $(((ended - started) / 1000000)) $(($(cat "$BENCH_DIR/rss") / 1024))

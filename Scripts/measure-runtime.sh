#!/bin/zsh
# Samples the app plus WebKit helpers that carry ParticleWall's bundle id.
# Usage: Scripts/measure-runtime.sh [seconds] [output.csv]
set -euo pipefail

DURATION="${1:-30}"
OUTPUT="${2:-}"
typeset -a TARGET_PIDS

if ! [[ "$DURATION" =~ '^[0-9]+$' ]] || (( DURATION < 1 )); then
  echo "Duration must be a positive number of seconds." >&2
  exit 2
fi

TARGET_PIDS=("${(@f)$(pgrep -x ParticleWall 2>/dev/null || true)}")
for helper in com.apple.WebKit.WebContent com.apple.WebKit.GPU com.apple.WebKit.Networking; do
  pid="$(pgrep -n -f "/${helper}.xpc/Contents/MacOS/${helper}" 2>/dev/null || true)"
  [[ -n "$pid" ]] && TARGET_PIDS+=("$pid")
done
TARGET_PIDS=("${(@u)TARGET_PIDS}")

sample() {
  local timestamp
  timestamp="$(date '+%Y-%m-%dT%H:%M:%S')"
  for pid in "${TARGET_PIDS[@]}"; do
    ps -p "$pid" -o pid=,pcpu=,rss=,command= 2>/dev/null || true
  done | awk -v timestamp="$timestamp" '
    NF >= 4 {
      pid=$1; cpu=$2; rss=$3;
      $1=$2=$3="";
      sub(/^ +/, "", $0);
      printf "%s,%s,%.2f,%.2f,\"%s\"\n",
             timestamp, pid, cpu, rss / 1024, $0
    }'
}

{
  echo 'timestamp,pid,cpu_percent,rss_mb,command'
  for ((second = 0; second < DURATION; second++)); do
    sample
    if (( second + 1 < DURATION )); then sleep 1; fi
  done
} | if [[ -n "$OUTPUT" ]]; then tee "$OUTPUT"; else cat; fi

if [[ -n "$OUTPUT" ]]; then
  awk -F, 'NR > 1 {
      if (!seen[$1]++) timestamps++;
      cpu[$1] += $3;
      rss[$1] += $4;
      rows++;
    }
    END {
      for (timestamp in cpu) {
        totalCPU += cpu[timestamp];
        totalRSS += rss[timestamp];
      }
      if (timestamps) printf "Intervals: %d  average total CPU: %.2f%%  average total RSS: %.2f MB\n",
                             timestamps, totalCPU / timestamps, totalRSS / timestamps;
      else print "No ParticleWall processes found.";
    }' "$OUTPUT"
fi

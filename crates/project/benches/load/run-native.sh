#!/bin/bash
# Runs the native measures of QA-7 in order, each after waiting up to $WAIT seconds (default 40)
# for a machine under 20 % CPU, and notes the load before and after in its log.
# Usage (from the repository root, Windows with Git Bash): WORK=/tmp/faulk-qa7 LOGS=./qa7-logs bash crates/project/benches/load/run-native.sh
# Logs are not kept in the repository: their numbers are copied into
# docs/spikes/QA-7-banc-de-charge-format-projet.md.
WORK=${WORK:-/tmp/faulk-qa7}
LOGS=${LOGS:-./qa7-logs}
WAIT=${WAIT:-40}
mkdir -p "$LOGS" "$WORK"
B="cargo bench -q -p project --bench load --"
D=$WORK/heavy

load() {
  powershell -NoProfile -Command "[math]::Round(((Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 2 -MaxSamples 3).CounterSamples.CookedValue | Measure-Object -Average).Average,0)" 2>/dev/null | tr -d '\r'
}

quiet() {
  start=$(date +%s)
  while true; do
    l=$(load)
    if [ -n "$l" ] && [ "$l" -lt 20 ]; then echo "quiet $l"; return; fi
    if [ $(($(date +%s) - start)) -gt "$WAIT" ]; then echo "NOT-QUIET $l"; return; fi
    sleep 10
  done
}

run() {
  name=$1
  shift
  echo "machine before: $(quiet) (CPU % over 6 s)" > "$LOGS/$name.log"
  "$@" >> "$LOGS/$name.log" 2>&1
  echo "machine after: CPU $(load) % over 6 s" >> "$LOGS/$name.log"
}

gen() { $B gen --dir "$D" --audio-mb "$1" ${2:+--blob-above $2} > /dev/null 2>&1; }

cargo bench -q -p project --bench load --no-run
gen 256
run open $B open --dir "$D" --runs 9
run save $B save --dir "$D" --runs 5
run mem $B mem --dir "$D"
run swap $B swap --dir "$D"
run swap-builtin $B swap --profile heavy-builtin
run swap-limits-notes $B swap --profile limits-notes
run swap-limits-lane $B swap --profile limits-lane
for mb in 256 512 1024; do
  gen $mb
  run vcs-lfs-$mb $B vcs --dir "$D" --lfs 1
done
gen 256
run vcs-plain-256 $B vcs --dir "$D" --lfs 0
gen 256 1000
run open-blob $B open --dir "$D" --runs 9
run save-blob $B save --dir "$D" --runs 5 --blob-above 1000
gen 256 1000
run vcs-blob $B vcs --dir "$D" --lfs 1 --blob-above 1000
run density $B density --dir "$WORK/density"
run dirs $B dirs --dir "$WORK/dirs"
$B stream --out "$WORK/heavy.stream"

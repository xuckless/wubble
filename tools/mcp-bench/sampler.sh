#!/bin/bash
# macOS: every ~2 s, each wubble process's CPU % (of one core) and memory (top), the whole GPU's
# utilisation (ioreg) and the free-memory percentage, tagged with the phase in OUTDIR/phase.
# Also logs each process's role (browser, gpu-process, renderer, utility) to OUTDIR/roles.csv.
#
#   bash sampler.sh OUTDIR [APP]      (APP defaults to /Applications/wubble.app; stop with kill)
out=$1
app=${2:-/Applications/wubble.app}
mkdir -p "$out"
[ -f "$out/phase" ] || echo idle > "$out/phase"
while true; do
  pids=$(pgrep -f "$app/Contents" | tr '\n' ' ')
  [ -z "$pids" ] && { sleep 2; continue; }
  ps -axo pid=,command= | grep "$app/Contents" | grep -v grep | awk '{t="main"; for(i=1;i<=NF;i++){if($i ~ /^--type=/){t=substr($i,8)} if($i ~ /^--utility-sub-type=/){t=t":"substr($i,20)}} print $1","t}' >> "$out/roles.csv"
  args=""
  for p in $pids; do args="$args -pid $p"; done
  ts=$(date +%s)
  ph=$(cat "$out/phase")
  # two samples, keep the second (the first has no delta)
  top -l 2 -s 1 -stats pid,cpu,mem $args 2>/dev/null |
    awk -v ts="$ts" -v ph="$ph" '/^Processes:/{n++} n==2 && $1 ~ /^[0-9]+$/ {print ts","ph","$1","$2","$3}' >> "$out/proc.csv"
  gpu=$(ioreg -r -d 1 -c IOAccelerator 2>/dev/null | grep -o '"Device Utilization %"=[0-9]*' | head -1 | cut -d= -f2)
  rend=$(ioreg -r -d 1 -c IOAccelerator 2>/dev/null | grep -o '"Renderer Utilization %"=[0-9]*' | head -1 | cut -d= -f2)
  free=$(memory_pressure 2>/dev/null | awk -F': ' '/free percentage/{print $2}' | tr -d '%')
  echo "$ts,$ph,$gpu,$rend,$free" >> "$out/sys.csv"
done

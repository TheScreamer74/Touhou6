#!/bin/zsh
# bot.sh <max-seconds> <tag> [focus]: drive driver.mjs (localhost:9377) with a blind
# shoot/sweep/bomb pattern until a new completed-run event is logged or time runs out.
# Not a player: it only produces natural game overs for the result-event checks.
d(){ curl -s "localhost:9377/$1" >/dev/null; }
cnt(){ curl -s localhost:9377/log | python3 -c "import json,sys;print(len(json.load(sys.stdin)['results']))"; }
hold=z; [ "$3" = focus ] && hold=z,Shift
c0=$(cnt); end=$(( $(date +%s) + $1 )); n=0
d "down?k=$hold"
while [ $(date +%s) -lt $end ]; do
  d "up?k=z"; d "down?k=z"
  d "down?k=ArrowLeft"; d "wait?ms=600"; d "up?k=ArrowLeft"
  d "down?k=ArrowDown"; d "wait?ms=150"; d "up?k=ArrowDown"
  d "down?k=ArrowRight"; d "wait?ms=600"; d "up?k=ArrowRight"
  n=$((n+1)); [ $((n % 3)) = 0 ] && d "key?k=x"
  [ $((n % 30)) = 0 ] && curl -s "localhost:9377/shot?name=$2-$n"
  [ "$(cnt)" != "$c0" ] && break
done
d "up?k=$hold"
curl -s "localhost:9377/shot?name=$2-end"; curl -s "localhost:9377/log"

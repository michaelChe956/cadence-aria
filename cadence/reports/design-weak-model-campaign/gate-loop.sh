#!/usr/bin/env bash
# Design baseline campaign: one provider per terminal, sequential within provider.
# Usage: bash gate-loop.sh <provider:pi|kimi_code|claude_code> <outDir>
set -u
PROVIDER="${1:?usage: bash gate-loop.sh <provider> <outDir>}"
OUT="${2:-/tmp/design-baseline}"
mkdir -p "$OUT"
echo "START $PROVIDER $(date -Is)" | tee -a "$OUT/loop-$PROVIDER.log"
for shape in 01 02 03 04 05 06; do
  for rep in 1 2; do
    dir="$OUT/$PROVIDER/$shape-rep$rep"
    if [ -f "$dir/result.json" ]; then
      echo "SKIP $PROVIDER/$shape-rep$rep (already done)" | tee -a "$OUT/loop-$PROVIDER.log"
      continue
    fi
    echo "=== $(date -Is) RUN $PROVIDER/$shape-rep$rep ===" | tee -a "$OUT/loop-$PROVIDER.log"
    timeout 1300 node run_campaign.mjs "$PROVIDER" "$shape" "$rep" "$OUT" \
      >> "$OUT/loop-$PROVIDER.log" 2>&1
    rc=$?
    echo "=== $(date -Is) DONE $PROVIDER/$shape-rep$rep rc=$rc ===" | tee -a "$OUT/loop-$PROVIDER.log"
    sleep 3
  done
done
echo "ALL_DONE $PROVIDER $(date -Is)" | tee -a "$OUT/loop-$PROVIDER.log"

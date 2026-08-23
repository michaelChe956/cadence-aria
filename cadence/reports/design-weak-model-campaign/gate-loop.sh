#!/usr/bin/env bash
# Run the Design weak-model baseline grid sequentially and resumably.
# A completed sample is defined solely by its result.json, matching the campaign
# artifact contract.  Defaults yield 3 providers × 6 shapes × 2 repetitions.
set -u

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
OUT_ROOT=${OUT_ROOT:-/tmp/design-weak-model-campaign}
NODE_BIN=${NODE_BIN:-node}
TIMEOUT_SECS=${TIMEOUT_SECS:-660}
DRY_RUN=${DRY_RUN:-0}
PROVIDERS=${PROVIDERS:-"claude_code kimi_code pi"}
SHAPES=${SHAPES:-"01 02 03 04 05 06"}
REPS=${REPS:-"1 2"}

mkdir -p "$OUT_ROOT"
LOG_FILE="$OUT_ROOT/loop.log"
DRY_RUN_ARG=()
if [[ "$DRY_RUN" == "1" ]]; then
  DRY_RUN_ARG=(--dry-run)
fi

for provider in $PROVIDERS; do
  for shape in $SHAPES; do
    for rep in $REPS; do
      sample_dir="$OUT_ROOT/$provider/$shape-rep$rep"
      if [[ -f "$sample_dir/result.json" ]]; then
        echo "SKIP $provider/$shape-rep$rep (result.json already exists)" | tee -a "$LOG_FILE"
        continue
      fi

      echo "=== $(date -Is) RUN $provider/$shape-rep$rep ===" | tee -a "$LOG_FILE"
      timeout "$TIMEOUT_SECS" "$NODE_BIN" "$SCRIPT_DIR/run_campaign.mjs" \
        "$provider" "$shape" "$rep" "$OUT_ROOT" "${DRY_RUN_ARG[@]}" \
        >>"$LOG_FILE" 2>&1
      rc=$?
      echo "=== $(date -Is) DONE $provider/$shape-rep$rep rc=$rc ===" | tee -a "$LOG_FILE"

      # Dry-run is parameter/corpus validation only: it creates no result.json;
      # stop after the requested subset instead of repeatedly revalidating it.
      if [[ "$DRY_RUN" == "1" ]]; then
        continue
      fi
      sleep 3
    done
  done
done

echo "ALL_DONE $(date -Is)" | tee -a "$LOG_FILE"

#!/usr/bin/env bash
# Format AMM list JSON into simplified structure using jq

INPUT_FILE="${1:-pools.json}"
OUTPUT_FILE="${2:-formatted-pools.json}"

jq '[.[] | {id: .amm_process, name: (.amm_name + " " + (.pool_fee_bps|tostring)), type: "amm"}]' "$INPUT_FILE" > "$OUTPUT_FILE"

echo "Formatted AMMs saved to $OUTPUT_FILE"

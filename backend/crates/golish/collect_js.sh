#!/bin/bash

# Recursive JavaScript collector for Vite-managed apps
# Usage: ./collect_js.sh BASE_URL OUT_DIR INITIAL_FILES...

set -euo pipefail

BASE="$1"
OUT="$2"
shift 2

mkdir -p "$OUT"

declare -A DONE
QUEUE=("$@")

while [ ${#QUEUE[@]} -gt 0 ]; do
    FILE="${QUEUE[0]}"
    QUEUE=("${QUEUE[@]:1}")
    
    [[ -n "${DONE[$FILE]}" ]] && continue
    DONE[$FILE]=1

    HTTP=$(curl -sLk -w "%{http_code}" -o "$OUT/$FILE" "$BASE/$FILE" 2>/dev/null || echo "000")
    [ "$HTTP" != "200" ] && echo "FAIL $HTTP $FILE" && continue

    echo "OK $FILE"

    # Find referenced JS files in the downloaded file
    grep -oE '\./[a-zA-Z0-9_./-]+-[a-f0-9]{6,10}\.js' "$OUT/$FILE" 2>/dev/null | sed 's|\./||' | sort -u | while read REF; do
        [[ -z "${DONE[$REF]}" ]] && QUEUE+=("$REF")
    done

done

echo "TOTAL: ${#DONE[@]} files"

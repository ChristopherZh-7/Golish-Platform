#!/usr/bin/env bash

BASE="http://8.138.179.62:8080/jse"
OUT="js-assets/8.138.179.62:8080"
mkdir -p "$OUT"

# Use a file to track done files
DONE_FILE="$OUT/.done"
[ -f "$DONE_FILE" ] && rm "$DONE_FILE"

QUEUE=("index-index-Bg1R1mAJ.js")

while [ ${#QUEUE[@]} -gt 0 ]; do
  FILE="${QUEUE[0]}"
  QUEUE=("${QUEUE[@]:1}")
  
  # Skip if already done
  grep -qx "$FILE" "$DONE_FILE" 2>/dev/null && continue
  echo "$FILE" >> "$DONE_FILE"
  
  HTTP=$(curl -sLk -w "%{http_code}" -o "$OUT/$FILE" "$BASE/$FILE")
  [ "$HTTP" != "200" ] && echo "FAIL $HTTP $FILE" && continue
  
  # Extract references and add to queue
  grep -oE 'index-[a-zA-Z0-9_-]+-[a-f0-9]{8}\.js' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | while read -r ref; do
    grep -qx "${ref#/jse/}" "$DONE_FILE" 2>/dev/null || QUEUE+=("$ref")
  done
  
  grep -oE 'index-[a-zA-Z0-9_-]+-[a-f0-9]{8}\.mjs' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | while read -r ref; do
    grep -qx "${ref#/jse/}" "$DONE_FILE" 2>/dev/null || QUEUE+=("$ref")
  done
  
  grep -oE 'vendor-[a-f0-9]{8}\.js' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | while read -r ref; do
    grep -qx "${ref#/jse/}" "$DONE_FILE" 2>/dev/null || QUEUE+=("$ref")
  done
  
  grep -oE 'chunk-[a-zA-Z0-9_-]+-[a-f0-9]{8}\.js' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | while read -r ref; do
    grep -qx "${ref#/jse/}" "$DONE_FILE" 2>/dev/null || QUEUE+=("$ref")
  done
  
done

echo "TOTAL: $(wc -l < "$DONE_FILE")"

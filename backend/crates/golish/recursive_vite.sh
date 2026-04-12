#!/usr/bin/env bash

BASE="http://8.138.179.62:8080/jse"
OUT="js-assets/8.138.179.62:8080"
mkdir -p "$OUT"
declare -A DONE QUEUE=("index-index-Bg1R1mAJ.js")

while [ ${#QUEUE[@]} -gt 0 ]; do
  FILE="${QUEUE[0]}"
  QUEUE=("${QUEUE[@]:1}")
  [[ -n "${DONE[$FILE]}" ]] && continue
  DONE[$FILE]=1
  
  HTTP=$(curl -sLk -w "%{http_code}" -o "$OUT/$FILE" "$BASE/$FILE")
  [ "$HTTP" != "200" ] && echo "FAIL $HTTP $FILE" && continue
  
  for REF in $(grep -oE 'index-[a-zA-Z0-9_-]+-[a-f0-9]{8}\.js' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | sort -u); do
    [[ -z "${DONE[$REF]}" ]] && QUEUE+=("$REF")
  done
  
  for REF in $(grep -oE 'index-[a-zA-Z0-9_-]+-[a-f0-9]{8}\.mjs' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | sort -u); do
    [[ -z "${DONE[$REF]}" ]] && QUEUE+=("$REF")
  done
  
  for REF in $(grep -oE 'vendor-[a-f0-9]{8}\.js' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | sort -u); do
    [[ -z "${DONE[$REF]}" ]] && QUEUE+=("$REF")
  done
  
  for REF in $(grep -oE 'chunk-[a-zA-Z0-9_-]+-[a-f0-9]{8}\.js' "$OUT/$FILE" 2>/dev/null | sed 's|^|/jse/|' | sort -u); do
    [[ -z "${DONE[$REF]}" ]] && QUEUE+=("$REF")
  done
  
done

echo "TOTAL: ${#DONE[@]} files"

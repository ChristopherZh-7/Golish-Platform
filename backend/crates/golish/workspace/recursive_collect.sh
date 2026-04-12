#!/bin/bash

# Recursive JS collection script for PRIORITY 2
# Downloads all JS files referenced by entry points and recursively discovered chunks.

TARGET_URL="http://8.138.179.62:8080"
OUTPUT_DIR="./golish_js_assets/8.138.179.62_8080"
ENTRY_POINTS=(
    "/jse/index-index-Bg1R1mAJ.js"
    "/_app.config.js?v=5.5.9-57335081"
)

mkdir -p "$OUTPUT_DIR"

# Use associative array for tracking (bash 4+)
# Initialize queue with entry points
QUEUE=()
for ENTRY in "${ENTRY_POINTS[@]}"; do
    QUEUE+=("$ENTRY")
done

# Track downloaded files
DONE_FILES=()
TOTAL_FILES=0
MAX_RETRIES=3
CYCLE=0

while [ $CYCLE -lt $MAX_RETRIES ]; do
    CYCLE=$((CYCLE + 1))
    echo "[+] Cycle $CYCLE: Discovering JS files..."
    NEW_FILES=0
    
    for FILE in "${QUEUE[@]}"; do
        # Skip if already processed
        SKIP=0
        for DONE in "${DONE_FILES[@]}"; do
            if [ "$DONE" = "$FILE" ]; then
                SKIP=1
                break
            fi
        done
        [ $SKIP -eq 1 ] && continue
        
        FILE_OUT="$OUTPUT_DIR/$FILE"
        FILE_URL="$TARGET_URL/$FILE"
        
        mkdir -p "$(dirname "$FILE_OUT")"
        
        HTTP_CODE=$(curl -sLk -o "$FILE_OUT" -w "%{http_code}" "$FILE_URL")
        
        if [ "$HTTP_CODE" = "200" ]; then
            echo "[+] Downloaded: $FILE_URL"
            DONE_FILES+=("$FILE")
            NEW_FILES=$((NEW_FILES + 1))
            TOTAL_FILES=$((TOTAL_FILES + 1))
            
            # Discover new references in this file
            if [[ "$FILE" == *".js"* ]]; then
                # Vite/Webpack/Next.js pattern: ./name-hash.js
                grep -oE '\[[^]]+\]|"[^"]+\.js"|\./[a-zA-Z0-9_./-]+-[a-f0-9]{6,10}\.js' "$FILE_OUT" 2>/dev/null | 
                sed -E 's/"\\[//g; s/\\]"//g; s/"//g; s|\./||g' | 
                sort -u | while read -r REF; do
                    # Add to queue if not already present
                    ADD=1
                    for Q in "${QUEUE[@]}"; do
                        [ "$Q" = "$REF" ] && ADD=0 && break
                    done
                    for D in "${DONE_FILES[@]}"; do
                        [ "$D" = "$REF" ] && ADD=0 && break
                    done
                    [ $ADD -eq 1 ] && QUEUE+=("$REF")
                done
            fi
        else
            echo "[!] FAIL $HTTP_CODE $FILE_URL"
        fi
    done
    
    if [ $NEW_FILES -eq 0 ]; then
        echo "[+] No new files discovered in cycle $CYCLE. Collection complete."
        break
    else
        echo "[+] Cycle $CYCLE: $NEW_FILES new files. Total: $TOTAL_FILES"
    fi
done

# Update index.json
cat > "$OUTPUT_DIR/index.json" << EOF
{
  "target_url": "$TARGET_URL",
  "collected_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "bundler": "vite",
  "strategy_used": "recursive",
  "files": [
EOF

# Generate file entries
for FILE in "${DONE_FILES[@]}"; do
    SIZE=$(stat -c%s "$OUTPUT_DIR/$FILE" 2>/dev/null || echo 0)
    echo "    {\"path\": \"$FILE\", \"url\": \"$TARGET_URL/$FILE\", \"size\": $SIZE, \"source\": \"recursive\"}," >> "$OUTPUT_DIR/index.json"
done

# Close JSON
cat >> "$OUTPUT_DIR/index.json" << 'EOF'
  ],
  "source_maps": [],
  "failed": [],
  "stats": {
    "total_files": '"$TOTAL_FILES"',
    "total_bytes": 0,
    "from_manifest": 0,
    "from_recursion": '"$TOTAL_FILES"',
    "from_ai_discovery": 0,
    "source_maps": 0,
    "failed": 0
  }
}
EOF

echo "[+] Total JS files collected: $TOTAL_FILES"

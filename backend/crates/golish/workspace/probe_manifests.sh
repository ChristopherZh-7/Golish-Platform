#!/bin/bash

# Manifest probing script for PRIORITY 1
# Probes known manifest paths in parallel and downloads all listed JS files if any manifest is found.

TARGET_URL="http://8.138.179.62:8080"
OUTPUT_DIR="./golish_js_assets/8.138.179.62_8080"
MANIFEST_PATHS=(
    "/.vite/manifest.json"
    "/manifest.json"
    "/asset-manifest.json"
    "/stats.json"
    "/_next/static/*/_buildManifest.js"
    "/_next/static/*/_ssgManifest.js"
    "/_nuxt/builds/latest.json"
    "/_nuxt/manifest.json"
    "/ngsw.json"
)

mkdir -p "$OUTPUT_DIR"

echo "[+] Probing for manifests..."

for MANIFEST_PATH in "${MANIFEST_PATHS[@]}"; do
    echo "[+] Probing: $MANIFEST_PATH"
    HTTP_CODE=$(curl -sLk -o "$OUTPUT_DIR/$(basename "$MANIFEST_PATH")" -w "%{http_code}" "$TARGET_URL$MANIFEST_PATH")
    
    if [ "$HTTP_CODE" = "200" ]; then
        echo "[+] Found manifest: $MANIFEST_PATH"
        
        # Parse manifest and extract JS files
        if [[ "$MANIFEST_PATH" == *".vite/manifest.json"* ]] || [[ "$MANIFEST_PATH" == *"/manifest.json"* ]]; then
            # Vite manifest
            grep -oE '"file":"[^"]+\.js"' "$OUTPUT_DIR/$(basename "$MANIFEST_PATH")" | cut -d'"' -f4 | while read -r JS_FILE; do
                JS_URL="$TARGET_URL$(dirname "$MANIFEST_PATH")/$JS_FILE"
                JS_OUT="$OUTPUT_DIR/$JS_FILE"
                mkdir -p "$(dirname "$JS_OUT")"
                curl -sLk -o "$JS_OUT" "$JS_URL"
                echo "[+] Downloaded: $JS_URL"
            done
        elif [[ "$MANIFEST_PATH" == *"/asset-manifest.json"* ]] || [[ "$MANIFEST_PATH" == *"/stats.json"* ]]; then
            # Webpack manifest
            grep -oE '"main\.js"|"[^"]+\.js"' "$OUTPUT_DIR/$(basename "$MANIFEST_PATH")" | grep -oE '"[^"]+\.js"' | cut -d'"' -f2 | while read -r JS_FILE; do
                JS_URL="$TARGET_URL/$(basename "$JS_FILE")"
                JS_OUT="$OUTPUT_DIR/$(basename "$JS_FILE")"
                curl -sLk -o "$JS_OUT" "$JS_URL"
                echo "[+] Downloaded: $JS_URL"
            done
        elif [[ "$MANIFEST_PATH" == *"_next"* ]]; then
            # Next.js manifest
            grep -oE '"[^"]+\.js"' "$OUTPUT_DIR/$(basename "$MANIFEST_PATH")" | grep -oE '"[^"]+\.js"' | cut -d'"' -f2 | while read -r JS_FILE; do
                JS_URL="$TARGET_URL$JS_FILE"
                JS_OUT="$OUTPUT_DIR/$JS_FILE"
                mkdir -p "$(dirname "$JS_OUT")"
                curl -sLk -o "$JS_OUT" "$JS_URL"
                echo "[+] Downloaded: $JS_URL"
            done
        elif [[ "$MANIFEST_PATH" == *"_nuxt"* ]]; then
            # Nuxt manifest
            grep -oE '"[^"]+\.js"' "$OUTPUT_DIR/$(basename "$MANIFEST_PATH")" | grep -oE '"[^"]+\.js"' | cut -d'"' -f2 | while read -r JS_FILE; do
                JS_URL="$TARGET_URL/_nuxt/$JS_FILE"
                JS_OUT="$OUTPUT_DIR/_nuxt/$JS_FILE"
                mkdir -p "$(dirname "$JS_OUT")"
                curl -sLk -o "$JS_OUT" "$JS_URL"
                echo "[+] Downloaded: $JS_URL"
            done
        elif [[ "$MANIFEST_PATH" == *"/ngsw.json"* ]]; then
            # Angular manifest
            grep -oE '"assetGroup":[^}]+\.js[^}]*' "$OUTPUT_DIR/$(basename "$MANIFEST_PATH")" | grep -oE '"url":"[^"]+"' | cut -d'"' -f4 | while read -r JS_FILE; do
                JS_URL="$TARGET_URL$JS_FILE"
                JS_OUT="$OUTPUT_DIR$JS_FILE"
                mkdir -p "$(dirname "$JS_OUT")"
                curl -sLk -o "$JS_OUT" "$JS_URL"
                echo "[+] Downloaded: $JS_URL"
            done
        fi
        
        # Generate manifest index
        cat > "$OUTPUT_DIR/index.json" << 'EOF'
{
  "target_url": "http://8.138.179.62:8080",
  "collected_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "bundler": "unknown",
  "strategy_used": "manifest",
  "files": [],
  "source_maps": [],
  "failed": [],
  "stats": {
    "total_files": 0,
    "total_bytes": 0,
    "from_manifest": 0,
    "from_recursion": 0,
    "from_ai_discovery": 0,
    "source_maps": 0,
    "failed": 0
  }
}
EOF
        
        exit 0
    fi
done

echo "[!] No manifest found."
exit 1
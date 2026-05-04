#!/usr/bin/env bash
# download_wordlists.sh - fetch a curated subset of SecLists wordlists into
# resources/wordlists/. Files are gitignored (see .gitignore). Run once on
# a fresh clone or whenever you want to refresh the local wordlist cache.
#
# Total download size: ~1 MB. See README.md in the destination dir for
# what each file is for and how to fetch the bigger ones (rockyou etc.).
#
# Usage:
#   ./scripts/download_wordlists.sh           # download default set
#   ./scripts/download_wordlists.sh --extra   # also pull rockyou.txt (~134 MB)
#   ./scripts/download_wordlists.sh --force   # re-download even if exists

set -euo pipefail

# ── locate destination ──────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DEST="$ROOT/resources/wordlists"
mkdir -p "$DEST"

# ── flags ──────────────────────────────────────────────────────────────
EXTRA=0
FORCE=0
for a in "$@"; do
  case "$a" in
    --extra) EXTRA=1 ;;
    --force) FORCE=1 ;;
    -h|--help)
      sed -n '2,16p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
  esac
done

# ── source: SecLists raw URLs (master branch) ──────────────────────────
SECLISTS="https://raw.githubusercontent.com/danielmiessler/SecLists/master"

# (relative_path, source_url)
FILES=(
  "common.txt|$SECLISTS/Discovery/Web-Content/common.txt"
  "raft-small-directories.txt|$SECLISTS/Discovery/Web-Content/raft-small-directories.txt"
  "raft-small-files.txt|$SECLISTS/Discovery/Web-Content/raft-small-files.txt"
  "quickhits.txt|$SECLISTS/Discovery/Web-Content/quickhits.txt"
  "subdomains-top1million-5000.txt|$SECLISTS/Discovery/DNS/subdomains-top1million-5000.txt"
  "subdomains-top1million-20000.txt|$SECLISTS/Discovery/DNS/subdomains-top1million-20000.txt"
  "burp-parameter-names.txt|$SECLISTS/Discovery/Web-Content/burp-parameter-names.txt"
  "api-endpoints.txt|$SECLISTS/Discovery/Web-Content/api/api-endpoints.txt"
  "top-usernames-shortlist.txt|$SECLISTS/Usernames/top-usernames-shortlist.txt"
  "xato-net-10m-usernames-dup-1k.txt|$SECLISTS/Usernames/xato-net-10-million-usernames-dup.txt"
  "passwords-top1k.txt|$SECLISTS/Passwords/Common-Credentials/10-million-password-list-top-1000.txt"
  "probable-v2-top1575.txt|$SECLISTS/Passwords/probable-v2-top1575.txt"
)

# Optional large files (only with --extra)
EXTRA_FILES=(
  "rockyou.txt|$SECLISTS/Passwords/Leaked-Databases/rockyou.txt.tar.gz"
  "fasttrack.txt|$SECLISTS/Passwords/Leaked-Databases/fasttrack.txt"
)

# ── helpers ────────────────────────────────────────────────────────────
ok=0
skip=0
fail=0
need_extract=()

fetch() {
  local name="$1"
  local url="$2"
  local out="$DEST/$name"
  if [[ -f "$out" && "$FORCE" -eq 0 ]]; then
    printf "  \033[33mskip\033[0m  %-50s (exists, use --force)\n" "$name"
    skip=$((skip + 1))
    return
  fi
  if curl --fail --silent --show-error -L --connect-timeout 10 -o "$out" "$url"; then
    local size
    size=$(wc -c < "$out" | tr -d ' ')
    printf "  \033[32mok\033[0m    %-50s (%s bytes)\n" "$name" "$size"
    ok=$((ok + 1))
    case "$name" in
      *.tar.gz) need_extract+=("$out") ;;
    esac
  else
    rm -f "$out"
    printf "  \033[31mfail\033[0m  %-50s\n" "$name"
    fail=$((fail + 1))
  fi
}

# ── run ────────────────────────────────────────────────────────────────
echo "Destination: $DEST"
echo "Default wordlists (~1 MB total):"
for line in "${FILES[@]}"; do
  IFS='|' read -r name url <<< "$line"
  fetch "$name" "$url"
done

if [[ "$EXTRA" -eq 1 ]]; then
  echo ""
  echo "Extra wordlists (large, --extra):"
  for line in "${EXTRA_FILES[@]}"; do
    IFS='|' read -r name url <<< "$line"
    fetch "$name" "$url"
  done
fi

# Auto-extract any .tar.gz we downloaded
for tgz in "${need_extract[@]:-}"; do
  [[ -z "$tgz" ]] && continue
  echo ""
  echo "Extracting $tgz ..."
  if tar -xzf "$tgz" -C "$DEST"; then
    rm -f "$tgz"
    echo "  done"
  else
    echo "  \033[31mextract failed\033[0m"
  fi
done

echo ""
echo "── Summary ──────────────────────────────────────────"
echo "  ok:   $ok"
echo "  skip: $skip (already present)"
echo "  fail: $fail"
echo ""
echo "Tip:"
echo "  - Run with --force to re-download everything"
echo "  - Run with --extra to also fetch rockyou.txt (~134 MB)"
echo "  - All files are gitignored (see .gitignore)"
echo "  - For full SecLists clone:  git clone https://github.com/danielmiessler/SecLists $DEST/SecLists"

exit "$fail"

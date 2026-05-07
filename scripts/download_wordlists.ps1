# download_wordlists.ps1 — Windows / cross-platform PowerShell port of
# scripts/download_wordlists.sh. Fetches a curated subset of SecLists
# wordlists into resources/wordlists/.
#
# Total download size: ~1 MB. See README.md in the destination dir for
# what each file is for and how to fetch the bigger ones (rockyou etc.).
#
# Usage:
#   pwsh ./scripts/download_wordlists.ps1            # download default set
#   pwsh ./scripts/download_wordlists.ps1 -Extra     # also pull rockyou.txt (~134 MB)
#   pwsh ./scripts/download_wordlists.ps1 -Force     # re-download even if exists

[CmdletBinding()]
param(
    [switch]$Extra,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

# Resolve project root (scripts/.. → repo root)
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root      = (Resolve-Path (Join-Path $ScriptDir '..')).Path
$Dest      = Join-Path $Root 'resources/wordlists'

if (-not (Test-Path $Dest)) {
    New-Item -ItemType Directory -Path $Dest | Out-Null
}

$SecLists = 'https://raw.githubusercontent.com/danielmiessler/SecLists/master'

# Default wordlists: (output_name; source_url) tuples
$Files = @(
    @{ Name = 'common.txt';                          Url = "$SecLists/Discovery/Web-Content/common.txt" }
    @{ Name = 'raft-small-directories.txt';          Url = "$SecLists/Discovery/Web-Content/raft-small-directories.txt" }
    @{ Name = 'raft-small-files.txt';                Url = "$SecLists/Discovery/Web-Content/raft-small-files.txt" }
    @{ Name = 'quickhits.txt';                       Url = "$SecLists/Discovery/Web-Content/quickhits.txt" }
    @{ Name = 'subdomains-top1million-5000.txt';     Url = "$SecLists/Discovery/DNS/subdomains-top1million-5000.txt" }
    @{ Name = 'subdomains-top1million-20000.txt';    Url = "$SecLists/Discovery/DNS/subdomains-top1million-20000.txt" }
    @{ Name = 'burp-parameter-names.txt';            Url = "$SecLists/Discovery/Web-Content/burp-parameter-names.txt" }
    @{ Name = 'api-endpoints.txt';                   Url = "$SecLists/Discovery/Web-Content/api/api-endpoints.txt" }
    @{ Name = 'top-usernames-shortlist.txt';         Url = "$SecLists/Usernames/top-usernames-shortlist.txt" }
    @{ Name = 'xato-net-10m-usernames-dup-1k.txt';   Url = "$SecLists/Usernames/xato-net-10-million-usernames-dup.txt" }
    @{ Name = 'passwords-top1k.txt';                 Url = "$SecLists/Passwords/Common-Credentials/10-million-password-list-top-1000.txt" }
    @{ Name = 'probable-v2-top1575.txt';             Url = "$SecLists/Passwords/probable-v2-top1575.txt" }
)

$ExtraFiles = @(
    @{ Name = 'rockyou.txt';   Url = "$SecLists/Passwords/Leaked-Databases/rockyou.txt.tar.gz" }
    @{ Name = 'fasttrack.txt'; Url = "$SecLists/Passwords/Leaked-Databases/fasttrack.txt" }
)

$ok = 0
$skip = 0
$fail = 0
$needExtract = New-Object System.Collections.Generic.List[string]

function Fetch([string]$Name, [string]$Url) {
    $out = Join-Path $Dest $Name

    if ((Test-Path $out) -and -not $Force) {
        Write-Host ("  skip  {0,-50} (exists, use -Force)" -f $Name) -ForegroundColor Yellow
        $script:skip++
        return
    }

    try {
        Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $out -TimeoutSec 30
        $size = (Get-Item $out).Length
        Write-Host ("  ok    {0,-50} ({1} bytes)" -f $Name, $size) -ForegroundColor Green
        $script:ok++
        if ($Name.ToLower().EndsWith('.tar.gz')) {
            $script:needExtract.Add($out)
        }
    }
    catch {
        Write-Host ("  fail  {0,-50}" -f $Name) -ForegroundColor Red
        if (Test-Path $out) { Remove-Item $out -Force -ErrorAction SilentlyContinue }
        $script:fail++
    }
}

Write-Host "Destination: $Dest"
Write-Host "Default wordlists (~1 MB total):"
foreach ($f in $Files) { Fetch $f.Name $f.Url }

if ($Extra) {
    Write-Host ''
    Write-Host 'Extra wordlists (large, -Extra):'
    foreach ($f in $ExtraFiles) { Fetch $f.Name $f.Url }
}

# Auto-extract any .tar.gz we downloaded (uses tar.exe, available on
# Windows 10 1803+ and all modern Linux/macOS systems).
foreach ($tgz in $needExtract) {
    Write-Host ''
    Write-Host "Extracting $tgz ..."
    try {
        & tar -xzf $tgz -C $Dest
        if ($LASTEXITCODE -eq 0) {
            Remove-Item $tgz -Force -ErrorAction SilentlyContinue
            Write-Host '  done'
        }
        else {
            Write-Host '  extract failed' -ForegroundColor Red
        }
    }
    catch {
        Write-Host "  extract failed: $_" -ForegroundColor Red
    }
}

Write-Host ''
Write-Host '── Summary ──────────────────────────────────────────'
Write-Host "  ok:   $ok"
Write-Host "  skip: $skip (already present)"
Write-Host "  fail: $fail"
Write-Host ''
Write-Host 'Tip:'
Write-Host '  - Run with -Force to re-download everything'
Write-Host '  - Run with -Extra to also fetch rockyou.txt (~134 MB)'
Write-Host '  - All files are gitignored (see .gitignore)'
Write-Host "  - For full SecLists clone:  git clone https://github.com/danielmiessler/SecLists $Dest/SecLists"

exit $fail

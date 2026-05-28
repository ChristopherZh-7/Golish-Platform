# Windows Support Guide

This document describes the current state of Windows support for
Golish-Platform and the workarounds for areas where Windows behaves
differently from macOS / Linux.

## TL;DR

- The Tauri app **builds and runs** on Windows 10/11 via the
  `Build Windows` GitHub Actions workflow. The bundle config now ships
  proper NSIS / WiX entries (per-user installer, English + Simplified
  Chinese language picker, downloadable WebView2 bootstrapper).
- Most Rust crates compile cleanly on Windows. The hard-coded `sh -c`
  shell executions in `golish-pipeline` and `golish-pentest-mcp` have
  been replaced with the cross-platform helper exposed by
  `golish_shell_exec` (see `cross_shell::build_shell_command`).
- Penetration-testing tools whose `install.method` is `homebrew` /
  `homebrew-cask` cannot be auto-installed on Windows yet — see the
  [Tools and Package Managers](#tools-and-package-managers) table for
  recommended alternatives.
- The embedded vector database (PostgreSQL via `pg-embed` 1.x +
  optional `pgvector`) downloads and runs on Windows, but if the
  `pgvector` extension is not present the platform automatically falls
  back to application-level vector search.

## Prerequisites for development

| Component | Required version | Notes |
|---|---|---|
| Windows | 10 1809+ or 11 | WebView2 runtime is fetched on demand |
| PowerShell | 5.1+ (7+ recommended) | `pwsh` is preferred for the bundled scripts |
| Rust | stable, MSVC toolchain | `rustup target add x86_64-pc-windows-msvc` |
| Node.js | 20 LTS | install via `winget install OpenJS.NodeJS.LTS` |
| pnpm | 9.x | `corepack enable && corepack prepare pnpm@9 --activate` |
| Visual Studio Build Tools | 2022 | Required by the MSVC linker |
| WebView2 runtime | latest | shipped with Edge; fetched if missing |

The `justfile` recipes assume **GNU Make** + **bash** semantics. On
Windows you can either:

1. Use the Windows Subsystem for Linux (WSL2) and run `just` recipes
   from there (recommended for full parity).
2. Run individual commands directly. The most common ones are:

```powershell
pnpm install
pnpm dev               # frontend dev server
cargo check            # from .\backend\
cargo run -p golish    # from .\backend\
```

## Wordlists

The Bash variant `scripts/download_wordlists.sh` is mirrored by
`scripts/download_wordlists.ps1` (same defaults, same `-Extra` /
`-Force` flags). Both write to `resources/wordlists/`.

```powershell
pwsh ./scripts/download_wordlists.ps1
pwsh ./scripts/download_wordlists.ps1 -Extra -Force
```

## Tools and Package Managers

`resources/toolsconfig/*.json` describes how each pentest tool is
acquired. The current `install.method` values are mapped to the
following Windows alternatives. The plan is to extend the schema with
an optional `install.windows` block; until then, install the tool
manually using the recommended command and let the tool-manager
auto-detect it via `which_executable`.

| Tool | macOS (current) | Windows recommendation |
|---|---|---|
| `nmap` | `brew install nmap` | `winget install -e --id Insecure.Nmap` |
| `nuclei` | `brew install nuclei` | `scoop install nuclei` (extras bucket) or [GitHub release](https://github.com/projectdiscovery/nuclei/releases) |
| `httpx` | `brew install httpx` | [GitHub release](https://github.com/projectdiscovery/httpx/releases) (`*_windows_amd64.zip`) |
| `katana` | `brew install katana` | [GitHub release](https://github.com/projectdiscovery/katana/releases) |
| `subfinder` | `brew install subfinder` | [GitHub release](https://github.com/projectdiscovery/subfinder/releases) |
| `gowitness` | `brew install gowitness` | [GitHub release](https://github.com/sensepost/gowitness/releases) |
| `gobuster` | `brew install gobuster` | [GitHub release](https://github.com/OJ/gobuster/releases) |
| `dalfox` | `brew install dalfox` | [GitHub release](https://github.com/hahwul/dalfox/releases) |
| `ffuf` | `brew install ffuf` | [GitHub release](https://github.com/ffuf/ffuf/releases) |
| `gau` / `waybackurls` | `brew install gau`/`waybackurls` | `go install` from the upstream repo, or GitHub release |
| `chisel` | `brew install chisel` | [GitHub release](https://github.com/jpillora/chisel/releases) |
| `amass` | `brew install amass` | [GitHub release](https://github.com/owasp-amass/amass/releases) |
| `nikto` | `brew install nikto` | [GitHub clone](https://github.com/sullo/nikto) (Perl runtime required) |
| `john` (Jumbo) | `brew install john-jumbo` | [Hashcat-style binaries](https://www.openwall.com/john/) |
| `hashcat` | `brew install hashcat` | [Hashcat releases](https://hashcat.net/hashcat/) |
| `hydra` | `brew install hydra` | [THC-Hydra Windows binaries](https://github.com/maaaaz/thc-hydra-windows/releases) |
| `masscan` | `brew install masscan` | [GitHub release](https://github.com/robertdavidgraham/masscan/releases) |
| `metasploit-framework` | `brew install metasploit` | [Metasploit Windows installer](https://www.metasploit.com/download) |
| `wireshark` | `brew install --cask wireshark` | `winget install -e --id WiresharkFoundation.Wireshark` |
| `sqlmap` | GitHub clone | GitHub clone (works as-is — Python required) |
| `responder` | GitHub clone | GitHub clone (works on Python ≥3.9) |
| `searchsploit` | GitHub clone | GitHub clone — runs under Git Bash / WSL |
| `netexec` / `impacket` / `bloodhound-python` | `pip install …` | `pip install …` (works as-is on Windows Python ≥3.10) |
| `wpscan` | `gem install wpscan` | `gem install wpscan` (Ruby ≥3.1; install Ruby via `winget install RubyInstallerTeam.RubyWithDevKit`) |

### Schema extension (planned)

Future tool descriptors will use the following shape so the
tool-manager can pick the right installer per platform:

```json
{
  "tool": {
    "install": {
      "method": "homebrew",
      "source": "nmap",
      "windows": { "method": "winget", "source": "Insecure.Nmap" },
      "linux":   { "method": "apt",    "source": "nmap" }
    }
  }
}
```

The frontend `useToolInstall` hook now correctly picks
`*.exe` / `*.msi` / `*windows*` assets from GitHub releases, so any
tool whose `install.method` is `github` already works on Windows.

## Embedded vector database

The platform uses `pg-embed` 1.x to manage a per-user PostgreSQL 17
instance.

- Cache directory: `%LOCALAPPDATA%\pg-embed\<os>\<arch>\<version>`
- Data directory: `%LOCALAPPDATA%\golish-platform\pgdata`
- The fetch URL goes through Maven Central. Behind a corporate proxy
  set `HTTPS_PROXY` before launching the app or configure the proxy in
  `Settings → Network`.

`pgvector` is optional. The shipped detector
(`golish-db/embedded/platform.rs::find_system_pgvector`) currently has
no Windows search path. Two options:

1. **Recommended for now**: skip native pgvector — the platform falls
   back to application-level cosine search automatically. This is the
   behaviour you get out-of-the-box.
2. **Native pgvector on Windows** (advanced): build pgvector against
   a system PostgreSQL 17 install, then drop `vector.dll`,
   `vector.control`, and the SQL extension files into the cache
   directory's `lib/postgresql/` and `share/postgresql/extension/`
   folders.

## CI

`.github/workflows/check.yml` now contains a `windows-check` job that
runs on every PR (`windows-latest`). It performs:

- frontend `pnpm check` + `pnpm typecheck`
- `cargo check --workspace --target x86_64-pc-windows-msvc`

Runtime tests still execute only on Linux because most pentest tools
need Unix shells and tooling. The smoke check ensures Windows
compilation regressions are caught early.

## Known limitations

- macOS-only terminal launchers (iTerm2, Warp, Kitty, Alacritty,
  WezTerm, Hyper, Tabby) in `golish-pentest::terminal`. Windows opens
  PowerShell with UTF-8 enabled. Windows Terminal / Alacritty
  integrations are not yet implemented.
- `golish-pentest::handlers::homebrew` and a few other Unix-only
  install helpers return early on Windows. Use the manual
  installation table above.

## Reporting Windows-specific bugs

Please include in the bug report:

- Windows version (`winver` output)
- The platform string visible in `Settings → About`
- The contents of `%LOCALAPPDATA%\golish-platform\logs\golish.log`
  (last 200 lines)
- A description of which command / panel triggered the issue

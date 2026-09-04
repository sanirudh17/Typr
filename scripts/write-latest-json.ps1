param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$NotesFile
)

# Writes src-tauri/target/release/bundle/nsis/latest.json for the Tauri updater.
#
# CRITICAL: this file MUST be written as raw UTF-8 bytes WITHOUT a byte-order mark.
# The @tauri-apps/plugin-updater JSON decoder rejects a BOM with
# "error decoding response body" (regression from v0.1.4, repeat-offender v0.1.7).
# Do NOT use PowerShell `Set-Content -Encoding UTF8` here — that always writes a BOM.
# Do NOT redirect `>` — that also writes the host console code page.
# [Text.UTF8Encoding]::new($false) is the only safe path on Windows.

$ErrorActionPreference = "Stop"

$sigPath = "src-tauri\target\release\bundle\nsis\Typr_${Version}_x64-setup.exe.sig"
if (-not (Test-Path $sigPath)) {
    throw "Signature file not found: $sigPath"
}
$sig = (Get-Content $sigPath -Raw).Trim()

$json = @{
    version   = $Version
    notes     = (Get-Content $NotesFile -Raw).Trim()
    pub_date  = ([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ'))
    platforms = @{
        'windows-x86_64' = @{
            signature = $sig
            url       = "https://github.com/sanirudh17/Typr/releases/download/v$Version/Typr_${Version}_x64-setup.exe"
        }
    }
}
$text = $json | ConvertTo-Json -Depth 5

$out = "src-tauri\target\release\bundle\nsis\latest.json"
[IO.File]::WriteAllText($out, $text, [Text.UTF8Encoding]::new($false))

# Self-check: assert BOM is absent. If you ever see this throw, the build pipeline
# regressed — do NOT upload the manifest until it's fixed.
$bytes = [IO.File]::ReadAllBytes($out)
if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
    throw "BOM detected in $out - refusing to publish."
}
Write-Output "Wrote $out ($($bytes.Length) bytes, BOM-free)."

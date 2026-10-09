param(
    [Parameter(Mandatory = $true)][datetimeoffset]$Since,
    [string]$LogPath = (Join-Path $env:APPDATA 'com.typr.app\typr.log'),
    [string]$OutputFile = (Join-Path $env:USERPROFILE 'Downloads\typr-review-logs.txt')
)
$ErrorActionPreference = 'Stop'
$sourcePath = [IO.Path]::GetFullPath($LogPath)
$targetPath = [IO.Path]::GetFullPath($OutputFile)
if ($sourcePath -eq $targetPath) { throw 'Choose an output file different from the application log.' }
if (Test-Path -LiteralPath $targetPath) { throw 'Output file already exists; choose a new filename.' }
$cutoff = $Since.ToUnixTimeSeconds()
$includeLine = $false
$lines = New-Object 'System.Collections.Generic.List[string]'
foreach ($line in [IO.File]::ReadLines($sourcePath)) {
    if ($line -match '^\[(\d+)\] ') {
        $includeLine = [long]$Matches[1] -ge $cutoff
    }
    # Stage transcripts can contain continuation lines (paragraphs/lists).
    if ($includeLine) { $lines.Add($line) }
}
if ($lines.Count -eq 0) { throw 'No log entries found after the supplied start time.' }
[IO.File]::WriteAllLines($targetPath, $lines, (New-Object Text.UTF8Encoding($false)))
Write-Output "Saved $($lines.Count) lines to $targetPath"
Write-Output 'The log contains dictated text. Review the selected session before sharing it.'

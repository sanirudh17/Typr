param(
    [string]$Destination = (Join-Path $PSScriptRoot '../src-tauri/binaries'),
    [string]$ArchivePath
)
$ErrorActionPreference = 'Stop'
# Keep the executable and all runtime DLLs from one verified official build.
# Partial DLL sets can exit before main() with STATUS_DLL_NOT_FOUND.
$release = 'b5454'
$expectedHash = 'AFEF0B881C500958921C3F5523B50E59EE2EC9B6F5CBD25B324C51ED308A957A'
if (Test-Path -LiteralPath $Destination) {
    $item = Get-Item -LiteralPath $Destination
    if (!$item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Destination must be a real, empty directory; linked directories are not supported.'
    }
    if (Get-ChildItem -LiteralPath $Destination -Force | Select-Object -First 1) {
        throw 'Destination is not empty. Choose a new directory to preserve existing binaries.'
    }
}
if (!$ArchivePath) {
    $ArchivePath = Join-Path ([IO.Path]::GetTempPath()) ('typr-whisper-' + [Guid]::NewGuid().ToString('N') + '.zip')
    & curl.exe --fail --location --silent --show-error --output $ArchivePath (
        'https://github.com/ggml-org/whisper.cpp/releases/download/' + $release + '/whisper-bin-win-cuda-12.4.0-x64.zip')
    if ($LASTEXITCODE -ne 0) { throw 'Whisper runtime download failed.' }
}
if ((Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash -ne $expectedHash) {
    throw 'Whisper runtime archive checksum mismatch.'
}
$unpack = Join-Path ([IO.Path]::GetTempPath()) ('typr-whisper-unpack-' + [Guid]::NewGuid().ToString('N'))
Expand-Archive -LiteralPath $ArchivePath -DestinationPath $unpack
$source = Join-Path $unpack 'Release'
foreach ($name in @('whisper-cli.exe', 'whisper-server.exe', 'whisper.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu-x64.dll', 'ggml-cuda.dll', 'cublas64_12.dll', 'cublasLt64_12.dll', 'cudart64_12.dll')) {
    if (!(Test-Path -LiteralPath (Join-Path $source $name) -PathType Leaf)) { throw ('Incomplete archive: ' + $name) }
}
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
$mapping = @{
    'whisper-cpp-x86_64-pc-windows-msvc.exe' = 'whisper-cli.exe'
    'whisper-cpp-cuda-x86_64-pc-windows-msvc.exe' = 'whisper-cli.exe'
    'whisper-server-cuda-x86_64-pc-windows-msvc.exe' = 'whisper-server.exe'
}
foreach ($dll in Get-ChildItem -LiteralPath $source -File -Filter '*.dll') { $mapping[$dll.Name] = $dll.Name }
foreach ($entry in $mapping.GetEnumerator()) {
    $inputStream = [IO.File]::OpenRead((Join-Path $source $entry.Value))
    try {
        $outputStream = [IO.File]::Open((Join-Path $Destination $entry.Key), [IO.FileMode]::CreateNew)
        try { $inputStream.CopyTo($outputStream) } finally { $outputStream.Dispose() }
    } finally { $inputStream.Dispose() }
}
foreach ($name in @('whisper-cpp-x86_64-pc-windows-msvc.exe', 'whisper-cpp-cuda-x86_64-pc-windows-msvc.exe', 'whisper-server-cuda-x86_64-pc-windows-msvc.exe')) {
    # Help is supported by both binaries. --version is not a CLI option in this
    # pinned build. Capture normal stderr without PowerShell 5 treating it as a
    # terminating NativeCommandError under ErrorActionPreference=Stop.
    $process = New-Object Diagnostics.Process
    $process.StartInfo.FileName = Join-Path $Destination $name
    $process.StartInfo.Arguments = '-h'
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    try {
        if (!$process.Start()) { throw ('Could not start runtime: ' + $name) }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (!$process.WaitForExit(30000)) {
            $process.Kill()
            throw ('Runtime load check timed out: ' + $name)
        }
        $help = $stdout.Result + $stderr.Result
        if ($process.ExitCode -ne 0 -or $help -notmatch '(?i)usage:') {
            throw ('Whisper runtime failed its load check: ' + $name)
        }
    } finally { $process.Dispose() }
}
Write-Output ('Complete Whisper runtime prepared in ' + [IO.Path]::GetFullPath($Destination))

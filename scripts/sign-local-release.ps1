param(
    [Parameter(Mandatory = $true)][string]$Version,
    [string]$CredentialDirectory = (Join-Path $env:LOCALAPPDATA 'Typr\signing\updater-2026-10-08')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$keyPath = Join-Path $CredentialDirectory 'updater.key'
$passwordPath = Join-Path $CredentialDirectory 'password.dpapi'
$oldKey = $env:TAURI_SIGNING_PRIVATE_KEY
$oldPath = $env:TAURI_SIGNING_PRIVATE_KEY_PATH
$oldPassword = $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD
try {
    $protected = [IO.File]::ReadAllBytes($passwordPath)
    $bytes = [Security.Cryptography.ProtectedData]::Unprotect($protected, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    $env:TAURI_SIGNING_PRIVATE_KEY = $null
    $env:TAURI_SIGNING_PRIVATE_KEY_PATH = $keyPath
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = [Text.Encoding]::UTF8.GetString($bytes)
    & node (Join-Path $PSScriptRoot 'sign-release.mjs') $Version
    if ($LASTEXITCODE -ne 0) { throw 'Release signing failed.' }
} finally {
    $env:TAURI_SIGNING_PRIVATE_KEY = $oldKey
    $env:TAURI_SIGNING_PRIVATE_KEY_PATH = $oldPath
    $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = $oldPassword
    if ($bytes) { [Array]::Clear($bytes, 0, $bytes.Length) }
}

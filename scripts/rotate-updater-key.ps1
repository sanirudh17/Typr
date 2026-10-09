param(
    [string]$CredentialDirectory = (Join-Path $env:LOCALAPPDATA 'Typr\signing\updater-2026-10-08')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Security
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$credentialRoot = [IO.Path]::GetFullPath($CredentialDirectory)
if ($credentialRoot.StartsWith($repoRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) -or $credentialRoot -eq $repoRoot) {
    throw 'Store updater credentials outside the repository.'
}
$ancestor = $credentialRoot
while ($ancestor) {
    if (Test-Path -LiteralPath (Join-Path $ancestor '.git')) {
        throw 'Store updater credentials outside every Git checkout, including other worktrees.'
    }
    $parent = [IO.Directory]::GetParent($ancestor)
    $ancestor = if ($parent) { $parent.FullName } else { $null }
}
if (Test-Path -LiteralPath $credentialRoot) {
    throw 'Credential directory already exists. Use a new directory; existing keys are never overwritten.'
}
# Restrict the directory before any secret is written; files inherit this ACL.
$acl = New-Object System.Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true, $false)
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
$rule = New-Object System.Security.AccessControl.FileSystemAccessRule($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
$acl.AddAccessRule($rule)
[IO.Directory]::CreateDirectory($credentialRoot, $acl) | Out-Null
$keyPath = Join-Path $credentialRoot 'updater.key'
$passwordBytes = New-Object byte[] 32
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
try { $rng.GetBytes($passwordBytes) } finally { $rng.Dispose() }
$password = [Convert]::ToBase64String($passwordBytes)
$utf8 = [Text.Encoding]::UTF8
$passwordUtf8 = $utf8.GetBytes($password)
try {
    $protected = [Security.Cryptography.ProtectedData]::Protect($passwordUtf8, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
} finally {
    [Array]::Clear($passwordUtf8, 0, $passwordUtf8.Length)
}
[IO.File]::WriteAllBytes((Join-Path $credentialRoot 'password.dpapi'), $protected)
$previousPassword = $env:TYPR_NEW_UPDATER_PASSWORD
try {
    $env:TYPR_NEW_UPDATER_PASSWORD = $password
    # The helper invokes Tauri in-process so the password never enters argv.
    # CLI output may describe key material. Capture it and never forward it.
    $output = & node (Join-Path $PSScriptRoot 'generate-updater-key.mjs') $keyPath 2>&1
    if ($LASTEXITCODE -ne 0) { throw 'Updater key generation failed; no key material was printed.' }
} finally {
    $env:TYPR_NEW_UPDATER_PASSWORD = $previousPassword
    [Array]::Clear($passwordBytes, 0, $passwordBytes.Length)
    $password = $null
    $output = $null
}
$publicKey = [IO.File]::ReadAllText($keyPath + '.pub').Trim()
$configPath = Join-Path $repoRoot 'src-tauri\tauri.conf.json'
$configText = [IO.File]::ReadAllText($configPath)
$configText = [regex]::Replace($configText, '"pubkey"\s*:\s*"[^"]+"', ('"pubkey": "' + $publicKey + '"'))
[IO.File]::WriteAllText($configPath, $configText, (New-Object Text.UTF8Encoding($false)))
Write-Output "Replacement key generated. Private credentials: $credentialRoot"
Write-Output 'Only the public key was written to tauri.conf.json. Back up the key and password securely.'

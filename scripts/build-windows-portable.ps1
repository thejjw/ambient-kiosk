param(
    [string]$OutputDirectory,
    [string]$RuntimeCab
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$version = (Get-Content -LiteralPath (Join-Path $repo 'package.json') -Raw | ConvertFrom-Json).version
$runtimeVersion = '154.0.4258.37'
$runtimeHash = '143DA7F7C4939FDDD3875ED918E44022D7EB87063BF912FE3E32DF37C6B0B8C3'
$runtimeUrl = 'https://msedge.sf.dl.delivery.mp.microsoft.com/filestreamingservice/files/b82d47e8-d146-4563-94d1-3a3176b25c0a/Microsoft.WebView2.FixedVersionRuntime.154.0.4258.37.x64.cab'
$cabName = "Microsoft.WebView2.FixedVersionRuntime.$runtimeVersion.x64.cab"
$artifactRoot = Join-Path (Split-Path -Parent $repo) 'artifacts'
if (-not $OutputDirectory) { $OutputDirectory = $artifactRoot }
$OutputDirectory = [System.IO.Path]::GetFullPath($OutputDirectory)
$cacheDirectory = Join-Path $artifactRoot 'webview2'
if (-not $RuntimeCab) { $RuntimeCab = Join-Path $cacheDirectory $cabName }
$RuntimeCab = [System.IO.Path]::GetFullPath($RuntimeCab)

if (-not [Environment]::Is64BitOperatingSystem) { throw 'Windows x64 is required.' }
if (-not (Get-Command bun -ErrorAction SilentlyContinue)) { throw 'Bun is required.' }
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if ((Test-Path -LiteralPath (Join-Path $cargoBin 'cargo.exe')) -and -not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $env:PATH = "$cargoBin;$env:PATH"
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Rust/Cargo is required.' }

$changes = & git -C $repo status --porcelain
if ($LASTEXITCODE -ne 0 -or $changes) { throw 'Commit or account for tracked and untracked repository changes before packaging.' }
$commit = (& git -C $repo rev-parse --short HEAD).Trim()
$name = "AmbientKiosk-$version-windows-x64-portable-$commit"
$zipPath = Join-Path $OutputDirectory "$name.zip"
if (Test-Path -LiteralPath $zipPath) { throw "Output already exists: $zipPath" }

New-Item -ItemType Directory -Force -Path $OutputDirectory, $cacheDirectory | Out-Null
if (-not (Test-Path -LiteralPath $RuntimeCab)) {
    Write-Host "Downloading Microsoft WebView2 Fixed Version $runtimeVersion..."
    Invoke-WebRequest -Uri $runtimeUrl -OutFile $RuntimeCab
}
if ((Get-FileHash -LiteralPath $RuntimeCab -Algorithm SHA256).Hash -ne $runtimeHash) {
    throw "WebView2 CAB hash mismatch: $RuntimeCab"
}

Push-Location $repo
try {
    & bun run tauri build --no-bundle --ci
    if ($LASTEXITCODE -ne 0) { throw 'Tauri release build failed.' }
} finally {
    Pop-Location
}

$exe = Join-Path $repo 'src-tauri\target\release\ambient-kiosk.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "Built executable is missing: $exe" }
$stage = Join-Path $OutputDirectory "stage-$([Guid]::NewGuid().ToString('N'))"
$runtimeStage = Join-Path $stage 'runtime'
New-Item -ItemType Directory -Path $runtimeStage | Out-Null
& expand.exe $RuntimeCab -F:* $runtimeStage | Out-Null
if ($LASTEXITCODE -ne 0) { throw 'WebView2 CAB extraction failed.' }
$runtimeFolder = Join-Path $runtimeStage "Microsoft.WebView2.FixedVersionRuntime.$runtimeVersion.x64"
if (-not (Test-Path -LiteralPath (Join-Path $runtimeFolder 'msedgewebview2.exe'))) {
    throw 'Extracted WebView2 runtime is incomplete.'
}
Get-ChildItem -LiteralPath $runtimeFolder -Force | Move-Item -Destination $runtimeStage
Remove-Item -LiteralPath $runtimeFolder

Copy-Item -LiteralPath $exe -Destination (Join-Path $stage 'ambient-kiosk.exe')
Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $stage
Copy-Item -LiteralPath (Join-Path $repo 'docs\WINDOWS_PORTABLE.md') -Destination (Join-Path $stage 'README.md')
Copy-Item -LiteralPath (Join-Path $repo 'docs\kiosk-config.example.json') -Destination $stage
$manifest = @(
    "Ambient Kiosk version: $version"
    "Source commit: $commit"
    "Rust: $(& rustc --version)"
    "Bun: $(& bun --version)"
    "WebView2 Fixed Version: $runtimeVersion"
    "WebView2 CAB SHA256: $runtimeHash"
    "WebView2 source: $runtimeUrl"
)
Set-Content -LiteralPath (Join-Path $stage 'BUILD-MANIFEST.txt') -Value $manifest -Encoding ascii
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zipPath -CompressionLevel Optimal
$zipHash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
Set-Content -LiteralPath "$zipPath.sha256" -Value "$zipHash  $([IO.Path]::GetFileName($zipPath))" -Encoding ascii
Write-Host "Portable package: $zipPath"
Write-Host "SHA256: $zipHash"

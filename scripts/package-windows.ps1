param([string]$Stage = "$PSScriptRoot\..\dist\windows\app")
$ErrorActionPreference = 'Stop'
Push-Location "$PSScriptRoot\.."
try {
    foreach ($required in @('gstreamer\bin', 'gstreamer\lib\gstreamer-1.0', 'ffmpeg.exe', 'third-party\gstreamer', 'third-party\ffmpeg')) {
        if (!(Test-Path (Join-Path $Stage $required))) { throw "Missing audited runtime input: $Stage\$required. See docs/BUILD.md" }
    }
    $configuration = & (Join-Path $Stage 'ffmpeg.exe') -buildconf 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { throw 'FFmpeg validation failed' }
    if ($configuration -match '--enable-(gpl|nonfree)') { throw 'Refusing GPL/nonfree FFmpeg in this package profile' }
    cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    Copy-Item target\release\clearcast.exe $Stage
    # Linked GStreamer DLLs must be discoverable before Rust's main executes.
    # Keep them alongside the executable; a PATH change inside main is too late.
    Copy-Item (Join-Path $Stage 'gstreamer\bin\*.dll') $Stage -Force
    New-Item -ItemType Directory -Force (Join-Path $Stage 'lib') | Out-Null
    Copy-Item (Join-Path $Stage 'gstreamer\lib\gstreamer-1.0') (Join-Path $Stage 'lib') -Recurse -Force
    if (Test-Path (Join-Path $Stage 'gstreamer\libexec')) {
        Copy-Item (Join-Path $Stage 'gstreamer\libexec') $Stage -Recurse -Force
    }
    Copy-Item LICENSE,THIRD_PARTY.md $Stage
    makensis "/DSTAGE=$Stage" packaging\windows.nsi
    if ($LASTEXITCODE -ne 0) { throw 'NSIS packaging failed' }
} finally { Pop-Location }

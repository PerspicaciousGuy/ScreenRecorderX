param(
    [string]$GStreamerRoot = $env:GSTREAMER_1_0_ROOT_MSVC_X86_64,
    [string]$FFmpegRoot = "$PSScriptRoot\..\dist\ffmpeg-windows",
    [string]$Stage = "$PSScriptRoot\..\dist\windows\app",
    [string]$Binary = "$PSScriptRoot\..\target\release\clearcast.exe",
    [string]$MsvcRedistRoot,
    [switch]$SkipBuild,
    [switch]$SkipInstaller
)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\windows-runtime.ps1"
Push-Location "$PSScriptRoot\.."
try {
    if (!$GStreamerRoot) { throw 'Set GSTREAMER_1_0_ROOT_MSVC_X86_64 or pass -GStreamerRoot (build machine only)' }
    $GStreamerRoot = (Resolve-Path -LiteralPath $GStreamerRoot).Path
    $FFmpegRoot = (Resolve-Path -LiteralPath $FFmpegRoot).Path
    $Stage = [IO.Path]::GetFullPath($Stage)
    # Never carry obsolete or unrelated plugins over from an earlier build.
    if (Test-Path -LiteralPath $Stage) { throw "Stage already exists: $Stage. Choose a new -Stage directory" }
    foreach ($required in @('bin\ffmpeg.exe', 'third-party\ffmpeg', 'third-party\gstreamer\licenses', 'third-party\gstreamer\cerbero-1.26.11.tar.gz')) {
        if (!(Test-Path -LiteralPath (Join-Path $FFmpegRoot $required))) { throw "Missing FFmpeg input: $required. See docs/BUILD.md" }
    }
    if (!$MsvcRedistRoot) {
        $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
        $visualStudio = & $vswhere -latest -products '*' -property installationPath
        if (!$visualStudio) { throw 'Visual Studio not found; pass -MsvcRedistRoot with the x64 VC CRT directory' }
        $crt = Get-ChildItem (Join-Path $visualStudio 'VC\Redist\MSVC') -Recurse -Filter 'msvcp140.dll' |
            Where-Object { $_.DirectoryName -match '\\x64\\Microsoft\.VC[^\\]+\.CRT$' } |
            Sort-Object FullName -Descending | Select-Object -First 1
        if (!$crt) { throw 'Visual C++ x64 redistributable DLLs not found' }
        $MsvcRedistRoot = $crt.DirectoryName
    }
    if (!$SkipBuild) {
        cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    }
    $Binary = (Resolve-Path -LiteralPath $Binary).Path
    $bin = New-Item -ItemType Directory -Path (Join-Path $Stage 'bin') -Force
    $plugins = New-Item -ItemType Directory -Path (Join-Path $Stage 'lib\gstreamer-1.0') -Force
    $scanner = New-Item -ItemType Directory -Path (Join-Path $Stage 'libexec\gstreamer-1.0') -Force
    $thirdParty = New-Item -ItemType Directory -Path (Join-Path $Stage 'third-party') -Force
    Copy-Item -LiteralPath $Binary -Destination $bin
    Copy-Item -LiteralPath (Join-Path $FFmpegRoot 'bin\ffmpeg.exe') -Destination $bin
    Copy-Item -LiteralPath (Join-Path $FFmpegRoot 'third-party\ffmpeg') -Destination $thirdParty -Recurse
    Copy-Item -LiteralPath (Join-Path $FFmpegRoot 'third-party\gstreamer') -Destination $thirdParty -Recurse
    # These are redistributable VC DLLs, not Windows system DLLs. Do not rely on
    # the build machine's installed Visual C++ runtime to satisfy app imports.
    foreach ($required in @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')) {
        if (!(Test-Path -LiteralPath (Join-Path $MsvcRedistRoot $required))) { throw "Missing VC runtime: $required" }
    }
    Get-ChildItem -LiteralPath $MsvcRedistRoot -Filter '*.dll' | Copy-Item -Destination $bin
    $pluginNames = @('coreelements', 'typefindfunctions', 'd3d11', 'mediafoundation',
        'wasapi2', 'audioconvert', 'audioresample', 'audiomixer', 'volume',
        'videoconvertscale', 'videorate', 'videocrop', 'videoparsersbad',
        'audioparsers', 'matroska', 'videotestsrc', 'audiotestsrc', 'openh264')
    foreach ($name in $pluginNames) {
        Copy-Item -LiteralPath (Join-Path $GStreamerRoot "lib\gstreamer-1.0\gst$name.dll") -Destination $plugins
    }
    foreach ($name in @('nvcodec', 'qsv')) {
        $plugin = Join-Path $GStreamerRoot "lib\gstreamer-1.0\gst$name.dll"
        if (Test-Path -LiteralPath $plugin) { Copy-Item -LiteralPath $plugin -Destination $plugins }
    }
    foreach ($helper in @('gspawn-win64-helper.exe', 'gspawn-win64-helper-console.exe')) {
        Copy-Item -LiteralPath (Join-Path $GStreamerRoot "bin\$helper") -Destination $bin
    }
    Copy-Item -LiteralPath (Join-Path $GStreamerRoot 'libexec\gstreamer-1.0\gst-plugin-scanner.exe') -Destination $scanner
    $inputs = @(Get-ChildItem -LiteralPath $bin -Filter '*.exe' | ForEach-Object FullName) +
        @((Join-Path $scanner 'gst-plugin-scanner.exe')) +
        @(Get-ChildItem -LiteralPath $plugins -Filter '*.dll' | ForEach-Object FullName)
    Copy-RuntimeDependencies -Files $inputs -RuntimeBin (Join-Path $GStreamerRoot 'bin') -Destination $bin
    # The scanner is a separate process with its own DLL search directory.
    Copy-RuntimeDependencies -Files @((Join-Path $scanner 'gst-plugin-scanner.exe')) -RuntimeBin (Join-Path $GStreamerRoot 'bin') -Destination $scanner
    Get-ChildItem -LiteralPath $MsvcRedistRoot -Filter '*.dll' | Copy-Item -Destination $scanner
    Copy-Item LICENSE,THIRD_PARTY.md -Destination $Stage
    Copy-Item scripts\build-ffmpeg.sh -Destination (Join-Path $thirdParty 'ffmpeg')
    @{
        gstreamer_version = '1.26.11'
        runtime = 'https://gstreamer.freedesktop.org/data/pkg/windows/1.26.11/msvc/'
        build_recipes = 'https://gitlab.freedesktop.org/gstreamer/cerbero/-/tree/1.26.11'
        source_archives = @('gstreamer', 'gst-plugins-base', 'gst-plugins-good', 'gst-plugins-bad') | ForEach-Object { "https://gstreamer.freedesktop.org/src/$_/$_-1.26.11.tar.xz" }
        plugins = @(Get-ChildItem -LiteralPath $plugins -Filter '*.dll' | ForEach-Object Name)
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $thirdParty 'gstreamer\BUILD-SOURCES.json') -Encoding UTF8
    $configurationPath = Join-Path $thirdParty 'ffmpeg\buildconf.txt'
    $configurationOutput = Join-Path $thirdParty 'ffmpeg\buildconf-stdout.txt'
    $ffmpegCheck = Start-Process -FilePath (Join-Path $bin 'ffmpeg.exe') -ArgumentList '-buildconf' -WindowStyle Hidden -Wait -PassThru -RedirectStandardError $configurationPath -RedirectStandardOutput $configurationOutput
    if ($ffmpegCheck.ExitCode -ne 0) { throw 'FFmpeg validation failed' }
    $configuration = Get-Content -LiteralPath $configurationPath -Raw
    if ($configuration -match '--enable-(gpl|nonfree)') { throw 'Refusing GPL/nonfree FFmpeg in this package profile' }
    & "$PSScriptRoot\test-windows-package.ps1" -Stage $Stage
    if (!$SkipInstaller) {
        makensis "/DSTAGE=$Stage" packaging\windows.nsi
        if ($LASTEXITCODE -ne 0) { throw 'NSIS packaging failed' }
    }
    Write-Output "Bundled Windows application staged at $Stage"
} finally { Pop-Location }

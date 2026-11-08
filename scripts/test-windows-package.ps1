param([Parameter(Mandatory)][string]$Stage)
$ErrorActionPreference = 'Stop'
$Stage = (Resolve-Path -LiteralPath $Stage).Path
$saved = @{}
$variables = @('PATH', 'GST_PLUGIN_PATH', 'GST_PLUGIN_PATH_1_0', 'GST_PLUGIN_SYSTEM_PATH',
    'GST_PLUGIN_SYSTEM_PATH_1_0', 'GST_PLUGIN_SCANNER', 'GST_PLUGIN_SCANNER_1_0', 'GST_REGISTRY_1_0')
$temp = Join-Path ([IO.Path]::GetTempPath()) ("clearcast-package-test-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    foreach ($name in $variables) {
        $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }
    $env:PATH = "$env:SystemRoot\System32;$env:SystemRoot"
    # Fresh cache, no SDK/plugin paths: the private bin/lib layout must work.
    $env:GST_REGISTRY_1_0 = Join-Path $temp 'registry.bin'
    $bin = Join-Path $Stage 'bin'
    . "$PSScriptRoot\windows-runtime.ps1"
    foreach ($file in @(Get-ChildItem $bin -Filter '*.exe') +
                      @(Get-ChildItem (Join-Path $Stage 'lib\gstreamer-1.0') -Filter '*.dll')) {
        foreach ($dependency in (Get-PeImports $file.FullName)) {
            if ($dependency -notmatch '^(api-ms-|ext-ms-)' -and
                !(Test-Path -LiteralPath (Join-Path $bin $dependency)) -and
                !(Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32\$dependency"))) {
                throw "Unbundled dependency $dependency required by $($file.Name)"
            }
        }
    }
    # Explicit synthetic software diagnostic exercises H.264/AAC and bundled remuxing.
    & (Join-Path $bin 'clearcast.exe') diagnose --software --seconds 3 --pause --output $temp
    if ($LASTEXITCODE -ne 0) { throw 'Bundled recording/remux smoke test failed' }
    $output = @(Get-ChildItem -LiteralPath $temp -Filter '*.mp4')
    if ($output.Count -ne 1 -or $output[0].Length -eq 0) { throw 'Bundled test produced no MP4' }
    & (Join-Path $bin 'ffmpeg.exe') -v error -i $output[0].FullName -map 0:v:0 -map 0:a:0 -c copy -f mp4 (Join-Path $temp 'verified.mp4')
    if ($LASTEXITCODE -ne 0) { throw 'Bundled MP4 lacks readable video/audio streams' }
    Write-Output 'Bundled recording, pause/resume and MP4 remux passed without SDK paths'
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    Write-Output "Package smoke-test files: $temp"
}

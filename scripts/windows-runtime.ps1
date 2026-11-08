# Read x64 PE import tables without requiring dumpbin on the packaging host.
function Get-PeImports([string]$Path) {
    $bytes = [IO.File]::ReadAllBytes($Path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    if ([BitConverter]::ToUInt32($bytes, $pe) -ne 0x4550) { throw "Invalid PE file: $Path" }
    $sections = [BitConverter]::ToUInt16($bytes, $pe + 6)
    $optionalSize = [BitConverter]::ToUInt16($bytes, $pe + 20)
    $optional = $pe + 24
    if ([BitConverter]::ToUInt16($bytes, $optional) -ne 0x20b) { throw "Expected x64 PE file: $Path" }
    $sectionTable = $optional + $optionalSize
    function Convert-Rva([uint32]$Rva) {
        for ($i = 0; $i -lt $sections; $i++) {
            $offset = $sectionTable + $i * 40
            $start = [BitConverter]::ToUInt32($bytes, $offset + 12)
            $size = [Math]::Max([BitConverter]::ToUInt32($bytes, $offset + 8), [BitConverter]::ToUInt32($bytes, $offset + 16))
            if ($Rva -ge $start -and $Rva -lt $start + $size) {
                return [int]($Rva - $start + [BitConverter]::ToUInt32($bytes, $offset + 20))
            }
        }
        throw "Invalid RVA in $Path"
    }
    # Normal and RVA-based x64 delay-load import descriptors.
    foreach ($table in @(@(1, 20, 12), @(13, 32, 4))) {
        $rva = [BitConverter]::ToUInt32($bytes, $optional + 112 + $table[0] * 8)
        if (!$rva) { continue }
        $offset = Convert-Rva $rva
        while ($true) {
            $nameRva = [BitConverter]::ToUInt32($bytes, $offset + $table[2])
            if (!$nameRva) { break }
            $nameOffset = Convert-Rva $nameRva
            $end = $nameOffset
            while ($bytes[$end] -ne 0) { $end++ }
            [Text.Encoding]::ASCII.GetString($bytes, $nameOffset, $end - $nameOffset)
            $offset += $table[1]
        }
    }
}

function Copy-RuntimeDependencies([string[]]$Files, [string]$RuntimeBin, [string]$Destination) {
    $pending = [Collections.Generic.Queue[string]]::new()
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $Files) { $pending.Enqueue($file) }
    while ($pending.Count) {
        $file = $pending.Dequeue()
        if (!$seen.Add([IO.Path]::GetFullPath($file))) { continue }
        foreach ($name in (Get-PeImports $file)) {
            $source = Join-Path $RuntimeBin $name
            if (Test-Path -LiteralPath $source -PathType Leaf) {
                Copy-Item -LiteralPath $source -Destination $Destination -Force
                $pending.Enqueue($source)
            } elseif (Test-Path -LiteralPath (Join-Path $Destination $name) -PathType Leaf) {
                $pending.Enqueue((Join-Path $Destination $name))
            } elseif ($name -notmatch '^(api-ms-|ext-ms-)' -and
                      !(Test-Path -LiteralPath (Join-Path $env:SystemRoot "System32\$name"))) {
                throw "Missing dependency $name required by $file"
            }
        }
    }
}

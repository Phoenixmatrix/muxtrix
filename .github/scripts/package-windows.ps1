param(
    [Parameter(Mandatory = $true)]
    [string]$MuxtrixPath,

    [Parameter(Mandatory = $true)]
    [string]$ControlPath,

    [Parameter(Mandatory = $true)]
    [string]$NoticesPath,

    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression

$inputs = [ordered]@{
    "muxtrix.exe"            = (Resolve-Path $MuxtrixPath).Path
    "muxtrixctl.exe"         = (Resolve-Path $ControlPath).Path
    "THIRD_PARTY_NOTICES.md" = (Resolve-Path $NoticesPath).Path
}
$versionInfo = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($inputs["muxtrix.exe"])
if ($versionInfo.ProductName -ne "Muxtrix") {
    throw "muxtrix.exe is missing its embedded Muxtrix Windows resources"
}
Add-Type -AssemblyName System.Drawing
$applicationIcon = [System.Drawing.Icon]::ExtractAssociatedIcon($inputs["muxtrix.exe"])
if ($null -eq $applicationIcon) {
    throw "muxtrix.exe is missing its embedded application icon"
}
$applicationIcon.Dispose()
$output = [System.IO.Path]::GetFullPath($OutputPath)
$parent = [System.IO.Path]::GetDirectoryName($output)
[System.IO.Directory]::CreateDirectory($parent) | Out-Null
if ([System.IO.File]::Exists($output)) {
    [System.IO.File]::Delete($output)
}

$stream = [System.IO.File]::Open(
    $output,
    [System.IO.FileMode]::CreateNew,
    [System.IO.FileAccess]::ReadWrite,
    [System.IO.FileShare]::None
)
try {
    $archive = [System.IO.Compression.ZipArchive]::new(
        $stream,
        [System.IO.Compression.ZipArchiveMode]::Create,
        $false
    )
    try {
        foreach ($entryName in $inputs.Keys) {
            $entry = $archive.CreateEntry(
                $entryName,
                [System.IO.Compression.CompressionLevel]::Optimal
            )
            $entry.LastWriteTime = [DateTimeOffset]::new(
                1980,
                1,
                1,
                0,
                0,
                0,
                [TimeSpan]::Zero
            )
            $source = [System.IO.File]::OpenRead($inputs[$entryName])
            try {
                $destination = $entry.Open()
                try {
                    $source.CopyTo($destination)
                } finally {
                    $destination.Dispose()
                }
            } finally {
                $source.Dispose()
            }
        }
    } finally {
        $archive.Dispose()
    }
} finally {
    $stream.Dispose()
}

if (-not [System.IO.File]::Exists($output)) {
    throw "Windows release archive was not created: $output"
}

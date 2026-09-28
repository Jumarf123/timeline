param([switch]$SkipUiTests)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE"
    }
}

Push-Location -LiteralPath $projectRoot
$previousExe = $env:TIMELINE_EXE
try {
    Invoke-Checked 'npm.cmd' @('ci')
    Invoke-Checked 'npm.cmd' @('run', 'build:web')
    Invoke-Checked 'cargo' @('fmt', '--all', '--', '--check')
    Invoke-Checked 'cargo' @('clippy', '--all-targets', '--locked', '--', '-D', 'warnings')
    Invoke-Checked 'cargo' @('test', '--release', '--locked')
    Invoke-Checked 'cargo' @('build', '--release', '--locked', '--bin', 'timeline')
    $env:TIMELINE_EXE = (Resolve-Path -LiteralPath 'target/release/timeline.exe').Path
    if ($SkipUiTests) {
        Invoke-Checked 'npm.cmd' @('run', 'test:i18n')
    } else {
        Invoke-Checked 'npm.cmd' @('test')
    }

    $manifest = Get-Content -LiteralPath 'Cargo.toml' -Raw
    $version = [regex]::Match($manifest, '(?m)^version = "([^"]+)"').Groups[1].Value
    if (-not $version) { throw 'Package version is missing' }
    $output = Join-Path $projectRoot 'dist'
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    Copy-Item -LiteralPath $env:TIMELINE_EXE -Destination (Join-Path $output 'Timeline.exe') -Force
    Copy-Item -LiteralPath 'docs/RELEASE-NOTES.md' -Destination (Join-Path $output 'release-notes.md') -Force
    Copy-Item -LiteralPath 'docs/FORMATS.md' -Destination (Join-Path $output 'formats-and-search.md') -Force
    $archive = Join-Path $output "Timeline-$version-Windows-x64.zip"
    Compress-Archive -LiteralPath @(
        (Join-Path $output 'Timeline.exe'),
        (Join-Path $output 'release-notes.md'),
        (Join-Path $output 'formats-and-search.md')
    ) -DestinationPath $archive -Force
    $checksums = @('Timeline.exe', (Split-Path -Leaf $archive)) | ForEach-Object {
        $hash = Get-FileHash -LiteralPath (Join-Path $output $_) -Algorithm SHA256
        '{0}  {1}' -f $hash.Hash.ToLowerInvariant(), $_
    }
    [System.IO.File]::WriteAllLines((Join-Path $output 'SHA256SUMS.txt'), $checksums)
    Write-Host "Release ready: $archive"
} finally {
    $env:TIMELINE_EXE = $previousExe
    Pop-Location
}

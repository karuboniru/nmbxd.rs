param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    $target = 'x86_64-pc-windows-msvc'
    if (!$SkipBuild) {
        cargo build --release --locked -p nmbxd --target $target
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    $metadataJson = cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo metadata' }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq 'nmbxd' }
    $version = $package.version
    if ($version -notmatch '^[A-Za-z0-9._+-]+$') { throw 'Invalid package version' }
    $name = "nmbxd-$version-windows-x86_64"
    $binary = Join-Path $metadata.target_directory "$target/release/nmbxd.exe"
    if (!(Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Missing executable: $binary" }

    New-Item -ItemType Directory -Force -Path dist | Out-Null
    $stage = Join-Path (Resolve-Path dist) ('.package-' + [guid]::NewGuid())
    $bundle = Join-Path $stage $name
    New-Item -ItemType Directory -Path "$bundle/licenses", "$bundle/docs" | Out-Null
    try {
        # Explicit allowlist: never copy credentials or the whole build tree.
        Copy-Item -LiteralPath $binary -Destination "$bundle/nmbxd.exe"
        Copy-Item README.md, .env.example -Destination $bundle
        Copy-Item docs/dark-mode.png, docs/settings.png -Destination "$bundle/docs"
        Copy-Item assets/fonts/OFL.txt "$bundle/licenses/NotoColorEmoji-OFL.txt"
        Copy-Item vendor/cosmic-text/unicode/LICENSE.txt "$bundle/licenses/Unicode.txt"
        Copy-Item vendor/cosmic-text/LICENSE-MIT "$bundle/licenses/cosmic-text-MIT.txt"
        Copy-Item vendor/cosmic-text/LICENSE-APACHE "$bundle/licenses/cosmic-text-APACHE.txt"
        Copy-Item vendor/gpui-component/LICENSE-APACHE "$bundle/licenses/gpui-component.txt"
        if (Test-Path LICENSE) { Copy-Item LICENSE $bundle }
        $archive = Join-Path (Resolve-Path dist) "$name.zip"
        Compress-Archive -LiteralPath $bundle -DestinationPath $archive -Force
        $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        # GNU sha256sum-compatible, without a BOM; verified again by release CI.
        "$hash  $name.zip" | Set-Content -LiteralPath "$archive.sha256" -Encoding ascii
        Write-Output "Created $archive and $archive.sha256"
    } finally {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
} finally {
    Pop-Location
}

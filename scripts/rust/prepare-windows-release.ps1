$ErrorActionPreference = 'Stop'
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
Set-Location -LiteralPath $root
if ($env:OS -ne 'Windows_NT') { throw 'Run this script on the Windows build host.' }
if (@(git status --porcelain).Count) { throw 'Commit the release source before preparing its package.' }
$sourceCommit = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot read the source commit.' }
$config = Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw -Encoding UTF8 | ConvertFrom-Json
$package = Get-Content -LiteralPath 'package.json' -Raw -Encoding UTF8 | ConvertFrom-Json
$version = $config.version
if ($package.version -ne $version) { throw 'Desktop versions do not match.' }
$runtime = Get-Content -LiteralPath 'native-adapter/dist/runtime.json' -Raw -Encoding UTF8 | ConvertFrom-Json
if ($runtime.platform -ne 'win32' -or $runtime.arch -ne 'x64') { throw 'Expected a native Windows x64 runtime.' }
$exe = Get-Item -LiteralPath 'target/release/ocs-desktop-rust.exe'
$installer = Get-Item -LiteralPath "target/release/bundle/nsis/OCS Rust_$($version)_x64-setup.exe"
$versionPattern = '^' + [regex]::Escape($version) + '(\.0)?$'
if ($exe.VersionInfo.ProductVersion -notmatch $versionPattern -or $installer.VersionInfo.ProductVersion -notmatch $versionPattern) {
    throw 'The executable or installer has an outdated product version.'
}
$scriptPath = Join-Path $root 'assets/ocs-rust.user.js'
$scriptVersion = [regex]::Match((Get-Content -LiteralPath $scriptPath -Raw -Encoding UTF8), '(?m)^//\s*@version\s+(\S+)\s*$').Groups[1].Value
$out = Join-Path $root "dist/release/v$version/windows-x64"
New-Item -ItemType Directory -Path $out -Force | Out-Null
$name = "OCS-Rust-Desktop-$version-windows-x64-setup.exe"
$outputFile = Join-Path $out $name
Copy-Item -LiteralPath $installer.FullName -Destination $outputFile
$record = [ordered]@{
    project = 'OCS Rust'; sourceCommit = $sourceCommit; desktopVersion = $version; scriptVersion = $scriptVersion
    userscriptSha256 = (Get-FileHash -LiteralPath $scriptPath -Algorithm SHA256).Hash.ToLower()
    target = [ordered]@{ platform = $runtime.platform; arch = $runtime.arch; node = $runtime.node; playwright = $runtime.playwright }
    checks = [ordered]@{ executableVersion = $exe.VersionInfo.ProductVersion; installerVersion = $installer.VersionInfo.ProductVersion }
    file = [ordered]@{ name = $name; bytes = (Get-Item -LiteralPath $outputFile).Length; sha256 = (Get-FileHash -LiteralPath $outputFile -Algorithm SHA256).Hash.ToLower() }
}
[IO.File]::WriteAllText((Join-Path $out 'windows-build.json'), ($record | ConvertTo-Json -Depth 5) + "`n", [Text.UTF8Encoding]::new($false))
Write-Output "Windows release package prepared: $out"

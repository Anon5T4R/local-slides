<#
.SYNOPSIS
  Build, sign and package Local Slides for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    localslides-<version>-windows-<arch>.msi            per-machine installer (WiX v5)
    localslides-<version>-windows-user-<arch>.msi       per-user installer, no admin rights
    localslides-<version>-windows-<arch>-portable.zip   localslides.exe + localslides-cli.exe

  The binaries link the C runtime statically (+crt-static), so neither the MSI nor the portable
  zip needs the Visual C++ redistributable. Signing is delegated to sign.ps1 (skipped with a
  warning when no signing secrets are set).

  Needs: Rust (MSVC toolchain + the target), the Windows SDK (rc.exe, signtool.exe),
  and WiX v5 with its UI and Util extensions (installer dialogs, "Launch Local Slides" on finish):
    dotnet tool install --global wix --version 5.0.2
    wix extension add -g WixToolset.UI.wixext/5.0.2
    wix extension add -g WixToolset.Util.wixext/5.0.2

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
  pwsh packaging/windows/package.ps1 -Arch x86 -SkipBuild
  pwsh packaging/windows/package.ps1 -Arch arm64     # cross-compiled; needs the MSVC ARM64 build tools
#>
param(
  [ValidateSet('x64', 'x86', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = $env:LOCALSLIDES_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
# MSI ProductVersion is numeric (major.minor.build); pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'x86' { 'i686-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

if (-not $env:LOCALSLIDES_BUILD_SHA) { $env:LOCALSLIDES_BUILD_SHA = (git -C $Root rev-parse HEAD 2>$null) }
if (-not $env:LOCALSLIDES_BUILD_DATE) { $env:LOCALSLIDES_BUILD_DATE = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd') }

Write-Output "Local Slides $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT: no VC++ redistributable needed. Scoped to the target so host build scripts and
  # proc-macros are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  # Fail the build (rather than warn) if the icon/VERSIONINFO can't be embedded.
  $env:LOCALSLIDES_REQUIRE_WINRES = '1'
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p localslides -p localslides-cli --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\release"

# Check both binaries' PE headers before packaging. Machine (COFF header) must match -Arch, so an
# x64 build can never ship labelled arm64. Subsystem (optional header): 2 = Windows GUI, 3 = console.
# The app must be GUI (no console window opens with it); the CLI must stay console so its output
# reaches the terminal.
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'x86' { 0x14C } 'arm64' { 0xAA64 } }
foreach ($check in @(@('localslides.exe', 2), @('localslides-cli.exe', 3))) {
  $h = Get-PeHeader (Join-Path $Bin $check[0])
  if ($h.Machine -ne $Machine) { throw "$($check[0]) is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
  if ($h.Subsystem -ne $check[1]) { throw "$($check[0]) has PE subsystem $($h.Subsystem), expected $($check[1])" }
  Write-Output "ok $($check[0]): $Arch, PE subsystem $($h.Subsystem)"
}

# The shortcuts name no MSI icon, so they show the icon embedded in localslides.exe (build.rs).
# -SkipBuild takes whatever binary is already there, which may come from a build without
# LOCALSLIDES_REQUIRE_WINRES=1 whose resource compilation failed: refuse an exe with no icon
# instead of shipping shortcuts with a generic one. ExtractIconEx with index -1 counts them.
$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Add-Type -Namespace LocalSlides -Name Shell -MemberDefinition '[DllImport("shell32.dll", CharSet = CharSet.Unicode)] public static extern uint ExtractIconEx(string file, int index, IntPtr[] large, IntPtr[] small, uint icons);'
$Icons = [LocalSlides.Shell]::ExtractIconEx((Join-Path $Bin 'localslides.exe'), -1, $null, $null, 0)
if ($Icons -eq 0) { throw 'localslides.exe has no embedded icon: rebuild it with LOCALSLIDES_REQUIRE_WINRES=1 (omit -SkipBuild)' }
Write-Output "ok localslides.exe: $Icons embedded icon group(s)"

Copy-Item (Join-Path $Bin 'localslides.exe'), (Join-Path $Bin 'localslides-cli.exe') $Stage
& (Join-Path $PSScriptRoot 'sign.ps1') (Join-Path $Stage 'localslides.exe') (Join-Path $Stage 'localslides-cli.exe')

# Required notices ship in every MSI, independently of optional craft-fonts.
$AppNotices = Join-Path $Stage 'app-notices'
New-Item -ItemType Directory -Force -Path $AppNotices | Out-Null
Copy-Item -LiteralPath (Join-Path $Root 'NOTICE'), (Join-Path $Root 'ATTRIBUTION.md') -Destination $AppNotices

# ---- font notices for the MSI ------------------------------------------------------------------
# The portable ZIP already carries these notices; stage the complete files before MSI creation too.
$Docs = Join-Path $Stage 'font-licences'
New-Item -ItemType Directory -Force -Path $Docs | Out-Null
$HasCraftFontLicences = 0
if ($env:CRAFT_FONTS_DIR) {
  foreach ($ofl in Get-ChildItem -Path (Join-Path $env:CRAFT_FONTS_DIR 'fonts\*\OFL.txt') -File -ErrorAction SilentlyContinue) {
    Copy-Item -LiteralPath $ofl.FullName -Destination (Join-Path $Docs "OFL-$($ofl.Directory.Name).txt")
    $HasCraftFontLicences = 1
  }
}
# ---- end font notices for the MSI --------------------------------------------------------------

# ---- MSI ---------------------------------------------------------------------------------------
# The installer's welcome page shows the license: build its RTF from LICENSE-MIT and LICENSE-APACHE
# so it never drifts from them. Hard-wrapped lines are rejoined into paragraphs so the text wraps
# to the dialog's width.
function ConvertTo-RtfText([string] $Text) {
  $sb = [System.Text.StringBuilder]::new()
  foreach ($ch in $Text.ToCharArray()) {
    $c = [int] $ch
    if ($c -eq 0x5C -or $c -eq 0x7B -or $c -eq 0x7D) { [void] $sb.Append('\').Append($ch) }
    elseif ($c -gt 127) { [void] $sb.Append("\u$(if ($c -gt 32767) { $c - 65536 } else { $c })?") }
    else { [void] $sb.Append($ch) }
  }
  return $sb.ToString()
}
$paragraphs = [System.Collections.Generic.List[string]]::new()
$paragraphs.Add('\b Local Slides\b0')
$paragraphs.Add((ConvertTo-RtfText 'Local Slides is dual-licensed under the MIT License or the Apache License, Version 2.0, at your option. Both license texts follow. Bundled fonts and other third-party material keep their own open licenses, listed in NOTICE and ATTRIBUTION.md at https://github.com/Anon5T4R/local-slides.'))
foreach ($file in 'LICENSE-MIT', 'LICENSE-APACHE') {
  $title = $true  # each file opens with its license's name: show it bold
  foreach ($para in ((Get-Content -Raw (Join-Path $Root $file)) -split '\r?\n\s*\r?\n')) {
    $joined = (($para -split '\r?\n') | ForEach-Object { $_.Trim() } | Where-Object { $_ }) -join ' '
    if (-not $joined) { continue }
    $paragraphs.Add($(if ($title) { "\b $(ConvertTo-RtfText $joined)\b0" } else { ConvertTo-RtfText $joined }))
    $title = $false
  }
}
$LicenseRtf = Join-Path $Stage 'license.rtf'
$rtf = '{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss\fcharset0 Tahoma;}}\viewkind4\uc1\pard\sa120\f0\fs16 ' +
  ($paragraphs -join "\par`r`n") + "\par`r`n}`r`n"
[System.IO.File]::WriteAllText($LicenseRtf, $rtf, [System.Text.Encoding]::ASCII)

# Two MSIs from one source: per-machine (Program Files, needs admin) and per-user (no admin, into
# %LOCALAPPDATA%\Programs; see localslides.wxs). The per-user name puts "user" before the arch so
# the per-machine MSI sorts first and the updater's asset-name parser ignores the user one.
$Msi = Join-Path $Dist "localslides-$Version-windows-$Arch.msi"
$UserMsi = Join-Path $Dist "localslides-$Version-windows-user-$Arch.msi"
foreach ($build in @(@($Msi, 'perMachine'), @($UserMsi, 'perUserOrMachine'))) {
  $out = $build[0]
  Invoke-Native "wix build ($($build[1]))" {
    wix build (Join-Path $PSScriptRoot 'localslides.wxs') -arch $Arch `
      -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
      -d "Version=$MsiVersion" -d "BinDir=$Stage" -d "IconPath=$(Join-Path $Root 'assets\app-icon\localslides.ico')" `
      -d "LicenseRtf=$LicenseRtf" `
      -d "AppNoticesDir=$AppNotices" -d "DocsDir=$Docs" -d "HasCraftFontLicences=$HasCraftFontLicences" -d "Scope=$($build[1])" `
      -o $out
  }
  # wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
  Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($out, '.wixpdb'))
  Invoke-Native "MSI shortcut icon validation (ICE50, $($build[1]))" {
    wix msi validate $out -ice ICE50 -intermediateFolder (Join-Path $Stage "msi-validation-$($build[1])")
  }
  & (Join-Path $PSScriptRoot 'sign.ps1') $out
}

# ---- portable zip ------------------------------------------------------------------------------
$Portable = Join-Path $TargetDir "windows-package\localslides-$Version-windows-$Arch-portable"
Remove-Item -Recurse -Force $Portable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Portable | Out-Null
Copy-Item (Join-Path $Stage '*.exe') $Portable
foreach ($f in 'README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ATTRIBUTION.md') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Portable }
}
# Builds made with craft-fonts (CRAFT_FONTS_DIR) embed its fonts: ship their licences.
if ($env:CRAFT_FONTS_DIR) {
  foreach ($ofl in Get-ChildItem -Path (Join-Path $env:CRAFT_FONTS_DIR 'fonts\*\OFL.txt') -ErrorAction SilentlyContinue) {
    Copy-Item $ofl.FullName (Join-Path $Portable "OFL-$($ofl.Directory.Name).txt")
  }
}
$Zip = Join-Path $Dist "localslides-$Version-windows-$Arch-portable.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
# Compress-Archive (PowerShell 5.1) writes entry names with backslashes; the updater's portable
# unpack refuses them ("The portable ZIP contains an unsafe path or link."), and a zip with '\'
# is ambiguous cross-platform besides. Build the archive by hand, every entry with '/'.
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$RootName = Split-Path $Portable -Leaf
$PortableFull = [System.IO.Path]::GetFullPath($Portable)
$ZipStream = [System.IO.File]::Open($Zip, [System.IO.FileMode]::Create)
$Archive = New-Object System.IO.Compression.ZipArchive($ZipStream, [System.IO.Compression.ZipArchiveMode]::Create)
try {
  Get-ChildItem -LiteralPath $Portable -Recurse -File | ForEach-Object {
    $Relative = [System.IO.Path]::GetFullPath($_.FullName).Substring($PortableFull.Length + 1) -replace '\\', '/'
    $Entry = $Archive.CreateEntry("$RootName/$Relative", [System.IO.Compression.CompressionLevel]::Optimal)
    $EntryStream = $Entry.Open()
    $FileStream = [System.IO.File]::OpenRead($_.FullName)
    try { $FileStream.CopyTo($EntryStream) } finally { $FileStream.Dispose(); $EntryStream.Dispose() }
  }
} finally {
  $Archive.Dispose()
  $ZipStream.Dispose()
}

# Smoke-test the CLI when this machine can run it. An ARM64 build made on an x64 runner can't run
# here.
$HostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($Arch -ne 'arm64' -or $HostArch -eq 'arm64') {
  Invoke-Native 'localslides-cli --version' { & (Join-Path $Stage 'localslides-cli.exe') --version }
} else {
  Write-Output "skipping localslides-cli --version: an $Arch build doesn't run on this $HostArch machine"
}
Get-Item $Msi, $UserMsi, $Zip | Format-Table Name, Length

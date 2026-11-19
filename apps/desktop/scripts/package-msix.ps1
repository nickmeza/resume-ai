[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PackageName,
    [Parameter(Mandatory)][string]$Publisher,
    [Parameter(Mandatory)][string]$PublisherDisplayName,
    [string]$BinaryPath,
    [string]$OutputDirectory
)

$ErrorActionPreference = 'Stop'
$desktop = Split-Path -Parent $PSScriptRoot
$repo = Split-Path -Parent (Split-Path -Parent $desktop)
if (!$BinaryPath) { $BinaryPath = Join-Path $repo 'target/release/resume-ai-desktop.exe' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $repo 'target/release/bundle/msix' }
$binary = (Get-Item -LiteralPath $BinaryPath).FullName
$config = Get-Content -LiteralPath (Join-Path $desktop 'package.json') -Raw | ConvertFrom-Json
if ($config.version -notmatch '^\d+\.\d+\.\d+$') {
    throw "MSIX packaging requires a stable desktop version. Found '$($config.version)'."
}
$version = "$($config.version).0"

$sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
$makeAppx = Get-ChildItem -LiteralPath $sdk -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64/MakeAppx.exe' } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
if (!$makeAppx) { throw 'MakeAppx.exe was not found. Install the Windows SDK.' }

$nameXml = [Security.SecurityElement]::Escape($PackageName)
$publisherXml = [Security.SecurityElement]::Escape($Publisher)
$publisherDisplayXml = [Security.SecurityElement]::Escape($PublisherDisplayName)
$manifest = @"
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
         IgnorableNamespaces="uap rescap">
  <Identity Name="$nameXml" Publisher="$publisherXml" Version="$version" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>Resume AI</DisplayName>
    <PublisherDisplayName>$publisherDisplayXml</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Resources>
    <Resource Language="en-us" />
    <Resource Language="es" />
  </Resources>
  <Applications>
    <Application Id="ResumeAI" Executable="resume-ai-desktop.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="Resume AI" Description="List and resume your AI coding-agent sessions"
                         BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png"
                         Square44x44Logo="Assets\Square44x44Logo.png" />
    </Application>
  </Applications>
  <Capabilities>
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
"@

$output = (New-Item -ItemType Directory -Path $OutputDirectory -Force).FullName
$stageName = 'msix-stage-' + [guid]::NewGuid().ToString('N')
$stage = (New-Item -ItemType Directory -Path (Join-Path $output $stageName)).FullName
try {
    $assets = (New-Item -ItemType Directory -Path (Join-Path $stage 'Assets')).FullName
    $icons = Join-Path $desktop 'src-tauri/icons'
    foreach ($logo in @('StoreLogo.png', 'Square150x150Logo.png', 'Square44x44Logo.png')) {
        Copy-Item -LiteralPath (Join-Path $icons $logo) -Destination $assets
    }
    Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'resume-ai-desktop.exe')
    Set-Content -LiteralPath (Join-Path $stage 'AppxManifest.xml') -Value $manifest -Encoding UTF8

    $package = Join-Path $output "resume-ai_${version}_x64.msix"
    & $makeAppx pack /d $stage /p $package /o
    if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed with exit code $LASTEXITCODE." }
    Write-Output $package
} finally {
    $resolvedStage = (Resolve-Path -LiteralPath $stage).Path
    if ($resolvedStage -eq (Join-Path $output $stageName)) {
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
}

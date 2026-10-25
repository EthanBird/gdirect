param(
    [string]$Version,
    [string]$InstallDir = "$env:LOCALAPPDATA\gdirect\bin",
    [string]$ArtifactDir,
    [switch]$WithMount,
    [switch]$NoPath
)
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
if (-not [Environment]::Is64BitOperatingSystem -or $env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    throw 'The Windows release requires an x86_64 PC.'
}
if (-not $Version) {
    $Version = (Invoke-RestMethod 'https://api.github.com/repos/mizorewww/gdirect/releases/latest').tag_name -replace '^v', ''
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid release version.' }
$Name = "gdirect-$Version-x86_64-pc-windows-gnu.zip"
$TempDir = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory $TempDir | Out-Null
try {
    if ($ArtifactDir) {
        Copy-Item (Join-Path $ArtifactDir $Name) $TempDir
        Copy-Item (Join-Path $ArtifactDir 'SHA256SUMS') $TempDir
    } else {
        $Base = "https://github.com/mizorewww/gdirect/releases/download/v$Version"
        Invoke-WebRequest "$Base/$Name" -OutFile (Join-Path $TempDir $Name) -UseBasicParsing
        Invoke-WebRequest "$Base/SHA256SUMS" -OutFile (Join-Path $TempDir 'SHA256SUMS') -UseBasicParsing
    }
    $Expected = $null
    foreach ($Line in (Get-Content (Join-Path $TempDir 'SHA256SUMS'))) {
        if ($Line.Trim() -match ('^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($Name) + '$')) { $Expected = $Matches[1] }
    }
    $Actual = (Get-FileHash (Join-Path $TempDir $Name) -Algorithm SHA256).Hash
    if (-not $Expected -or $Expected -ne $Actual) { throw 'Release checksum verification failed.' }
    Expand-Archive (Join-Path $TempDir $Name) (Join-Path $TempDir 'extracted')
    $Root = Join-Path $TempDir "extracted\gdirect-$Version-x86_64-pc-windows-gnu"
    New-Item -ItemType Directory -Force $InstallDir | Out-Null
    Copy-Item "$Root\*" $InstallDir -Recurse -Force
    if ($WithMount) {
        $Rclone = Get-Command rclone -ErrorAction SilentlyContinue
        $NeedsRclone = -not $Rclone
        if ($Rclone) {
            $Line = (& $Rclone.Source version | Select-Object -First 1)
            if ($Line -match '^rclone v(\d+\.\d+\.\d+)') { $NeedsRclone = [version]$Matches[1] -lt [version]'1.75.2' }
            else { $NeedsRclone = $true }
        }
        $HasWinFsp = (Test-Path 'HKLM:\SOFTWARE\WinFsp') -or (Test-Path 'HKLM:\SOFTWARE\WOW6432Node\WinFsp')
        if (($NeedsRclone -or -not $HasWinFsp) -and -not (Get-Command winget -ErrorAction SilentlyContinue)) {
            throw 'Mount dependencies need winget. Install/update Microsoft App Installer, then rerun with -WithMount.'
        }
        $Packages = @()
        if ($NeedsRclone) { $Packages += 'Rclone.Rclone' }
        if (-not $HasWinFsp) { $Packages += 'WinFsp.WinFsp' }
        foreach ($Package in $Packages) {
            & winget install --id $Package --exact --accept-package-agreements --accept-source-agreements
            if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 3010) { throw "Failed to install $Package ($LASTEXITCODE)." }
        }
    }
    if (-not $NoPath) {
        $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (($UserPath -split ';') -notcontains $InstallDir) {
            [Environment]::SetEnvironmentVariable('Path', "$InstallDir;$UserPath", 'User')
        }
        $env:Path = "$InstallDir;$env:Path"
    }
    & (Join-Path $InstallDir 'gdirect.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'Installed executable failed its smoke test.' }
    Write-Host "Installed in $InstallDir. Open a new terminal to refresh PATH."
    Write-Host 'Next: create a Google OAuth client (one time), then sign in:'
    Write-Host '  https://github.com/mizorewww/gdirect/blob/main/docs/configuration/README.md'
    Write-Host '  gdirect login --client-secret C:\path\to\client_secret.json'
} finally {
    Remove-Item $TempDir -Recurse -Force
}

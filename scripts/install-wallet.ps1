param(
    [string]$InstallDir = "",
    [string]$SourceDir = "",
    [switch]$NoDeps,
    [switch]$NoHardware,
    [switch]$Yes,
    [switch]$DryRun,
    [switch]$NoShortcut,
    [switch]$Help
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoUrl = "https://github.com/triamazikamno/railoxide.git"
$Branch = "main"
$Toolchain = "1.91.0"
$Target = "x86_64-pc-windows-msvc"
$SqliteVersion = "3530200"

function Show-Usage {
    @"
RailOxide Windows source installer.

This installer builds the latest main branch from source. It does not install
prebuilt binaries.

Usage:
  irm https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet.ps1 | iex

Options:
  -InstallDir PATH       Install wallet.exe and sqlite3.dll under PATH
  -SourceDir PATH        Use PATH for the managed source checkout
  -NoDeps                Do not install missing system dependencies
  -NoHardware            Build without hardware-wallet support
  -Yes                   Do not prompt before dependency installs/downloads
  -DryRun                Print what would happen without changing anything
  -NoShortcut            Do not create a Start Menu shortcut
  -Help                  Show this help

Inspect-first flow:
  iwr https://raw.githubusercontent.com/triamazikamno/railoxide/main/scripts/install-wallet.ps1 -OutFile install-wallet.ps1
  notepad .\install-wallet.ps1
  powershell -ExecutionPolicy Bypass -File .\install-wallet.ps1

"@
}

function Write-Step {
    param([string]$Message)
    Write-Host "==> $Message"
}

function Stop-Install {
    param([string]$Message)
    throw $Message
}

function Confirm-Action {
    param([string]$Prompt)

    if ($Yes) {
        return $true
    }

    try {
        $answer = Read-Host "$Prompt [Y/n]"
    } catch {
        Stop-Install "$Prompt Pass -Yes to continue non-interactively."
    }

    if ([string]::IsNullOrWhiteSpace($answer)) {
        return $true
    }

    $normalized = $answer.Trim().ToLowerInvariant()
    return $normalized -eq "y" -or $normalized -eq "yes"
}

function Invoke-External {
    param(
        [string]$FilePath,
        [string[]]$ArgumentList = @()
    )

    if ($DryRun) {
        $quotedArgs = $ArgumentList | ForEach-Object {
            if ($_ -match '\s') { '"{0}"' -f $_ } else { $_ }
        }
        Write-Host "+ $FilePath $($quotedArgs -join ' ')"
        return
    }

    & $FilePath @ArgumentList
    if ($LASTEXITCODE -ne 0) {
        Stop-Install "$FilePath exited with status $LASTEXITCODE"
    }
}

function Invoke-Cmd {
    param([string]$Command)
    Invoke-External "cmd.exe" @("/c", $Command)
}

function Test-Command {
    param([string]$Name)
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Refresh-Path {
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $extraPaths = @(
        "C:\Program Files\LLVM\bin",
        "C:\Program Files\Git\cmd",
        "C:\Program Files\CMake\bin",
        "C:\Program Files\protobuf\bin"
    )
    $env:Path = (@($machinePath, $userPath) + $extraPaths | Where-Object { $_ }) -join ";"
}

function Get-VsDevCmdPath {
    $programFilesX86 = ${env:ProgramFiles(x86)}
    if ([string]::IsNullOrWhiteSpace($programFilesX86)) {
        $programFilesX86 = Join-Path $env:SystemDrive "Program Files (x86)"
    }

    $candidates = @(
        "$programFilesX86\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat",
        "$programFilesX86\Microsoft Visual Studio\2022\Community\Common7\Tools\VsDevCmd.bat",
        "$programFilesX86\Microsoft Visual Studio\2022\Professional\Common7\Tools\VsDevCmd.bat",
        "$programFilesX86\Microsoft Visual Studio\2022\Enterprise\Common7\Tools\VsDevCmd.bat"
    )

    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return $candidate
        }
    }

    return $null
}

function Install-WingetPackage {
    param(
        [string]$Id,
        [string]$Name,
        [string]$Override = ""
    )

    Write-Step "installing $Name with winget"
    $args = @(
        "install",
        "--id", $Id,
        "--exact",
        "--source", "winget",
        "--silent",
        "--accept-source-agreements",
        "--accept-package-agreements",
        "--disable-interactivity"
    )

    if (-not [string]::IsNullOrWhiteSpace($Override)) {
        $args += @("--override", $Override)
    }

    Invoke-External "winget" $args
}

function Ensure-Dependencies {
    Refresh-Path

    $missing = New-Object System.Collections.Generic.List[string]
    if (-not (Test-Command "git")) { $missing.Add("Git") }
    if (-not (Test-Command "rustup")) { $missing.Add("Rustup") }
    if (-not (Test-Command "cmake")) { $missing.Add("CMake") }
    if (-not (Test-Command "protoc")) { $missing.Add("Protobuf") }
    if (-not (Test-Command "clang")) { $missing.Add("LLVM") }
    if (-not (Get-VsDevCmdPath)) { $missing.Add("Visual Studio Build Tools") }

    if ($missing.Count -eq 0) {
        return
    }

    if ($NoDeps) {
        Stop-Install "missing dependencies: $($missing -join ', '); rerun without -NoDeps or install them manually"
    }

    if (-not (Test-Command "winget")) {
        Stop-Install "winget is required for automatic dependency installation"
    }

    if (-not (Confirm-Action "Install missing dependencies with winget: $($missing -join ', ')?")) {
        Stop-Install "dependencies are required to build RailOxide"
    }

    if ($missing -contains "Git") {
        Install-WingetPackage "Git.Git" "Git"
    }
    if ($missing -contains "Rustup") {
        Install-WingetPackage "Rustlang.Rustup" "Rustup"
    }
    if ($missing -contains "CMake") {
        Install-WingetPackage "Kitware.CMake" "CMake"
    }
    if ($missing -contains "Protobuf") {
        Install-WingetPackage "Google.Protobuf" "Protobuf"
    }
    if ($missing -contains "LLVM") {
        Install-WingetPackage "LLVM.LLVM" "LLVM"
    }
    if ($missing -contains "Visual Studio Build Tools") {
        $override = "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --add Microsoft.VisualStudio.Component.VC.Tools.ARM64 --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 --add Microsoft.VisualStudio.Component.Windows11SDK.26100 --add Microsoft.VisualStudio.Component.VC.CMake.Project --add Microsoft.VisualStudio.Component.VC.Redist.14.Latest"
        Install-WingetPackage "Microsoft.VisualStudio.2022.BuildTools" "Visual Studio Build Tools" $override
    }

    Refresh-Path
}

function Ensure-RequiredTools {
    Refresh-Path
    foreach ($tool in @("git", "rustup", "cmake", "protoc", "clang")) {
        if (-not (Test-Command $tool)) {
            Stop-Install "$tool is required; install it or rerun without -NoDeps"
        }
    }

    if (-not (Get-VsDevCmdPath)) {
        Stop-Install "Visual Studio Build Tools are required; install them or rerun without -NoDeps"
    }
}

function Ensure-RustToolchain {
    Write-Step "installing Rust toolchain $Toolchain"
    Invoke-External "rustup" @("toolchain", "install", $Toolchain)
    Invoke-External "rustup" @("target", "add", $Target, "--toolchain", $Toolchain)
}

function Get-ManagedMarkerPath {
    return (Join-Path $SourceDir ".railoxide-installer-managed")
}

function Ensure-SourceCheckout {
    $marker = Get-ManagedMarkerPath
    $parent = Split-Path -Parent $SourceDir

    if (-not (Test-Path -LiteralPath $SourceDir)) {
        Write-Step "cloning RailOxide main into $SourceDir"
        if ($DryRun) {
            Write-Host "+ New-Item -ItemType Directory -Force $parent"
            Write-Host "+ git clone --branch $Branch --single-branch $RepoUrl $SourceDir"
            Write-Host "+ write installer marker $marker"
        } else {
            New-Item -ItemType Directory -Path $parent -Force | Out-Null
            Invoke-External "git" @("clone", "--branch", $Branch, "--single-branch", $RepoUrl, $SourceDir)
            Set-Content -LiteralPath $marker -Value $RepoUrl -Encoding ASCII
        }
        return
    }

    if (-not (Test-Path -LiteralPath $marker)) {
        Stop-Install "$SourceDir already exists and is not marked as installer-managed; pass -SourceDir to use another path"
    }

    if (-not (Test-Path -LiteralPath (Join-Path $SourceDir ".git"))) {
        Stop-Install "$SourceDir exists but is not a Git checkout"
    }

    $status = (& git -C $SourceDir status --porcelain --untracked-files=no)
    if ($LASTEXITCODE -ne 0) {
        Stop-Install "git status failed in $SourceDir"
    }
    if (-not [string]::IsNullOrWhiteSpace(($status -join "`n"))) {
        Stop-Install "$SourceDir has local modifications; resolve them or use -SourceDir with a fresh path"
    }

    Write-Step "updating RailOxide main in $SourceDir"
    Invoke-External "git" @("-C", $SourceDir, "fetch", "origin", $Branch)
    Invoke-External "git" @("-C", $SourceDir, "checkout", $Branch)
    Invoke-External "git" @("-C", $SourceDir, "merge", "--ff-only", "origin/$Branch")
}

function Get-SourceCommit {
    $commit = (& git -C $SourceDir rev-parse HEAD)
    if ($LASTEXITCODE -ne 0) {
        Stop-Install "git rev-parse failed in $SourceDir"
    }
    return $commit.Trim()
}

function Ensure-SqliteForLinking {
    $sqliteRoot = Join-Path $SourceDir ".deps\sqlite-x64"
    $dllZip = Join-Path $sqliteRoot "sqlite-dll-win-x64-$SqliteVersion.zip"
    $amalgamationZip = Join-Path $sqliteRoot "sqlite-amalgamation-$SqliteVersion.zip"
    $dllUrl = "https://www.sqlite.org/2026/sqlite-dll-win-x64-$SqliteVersion.zip"
    $amalgamationUrl = "https://www.sqlite.org/2026/sqlite-amalgamation-$SqliteVersion.zip"
    $sqliteLib = Join-Path $sqliteRoot "sqlite3.lib"
    $sqliteDef = Join-Path $sqliteRoot "sqlite3.def"

    Write-Step "preparing SQLite $SqliteVersion for Windows linking"
    if ($DryRun) {
        Write-Host "+ New-Item -ItemType Directory -Force $sqliteRoot"
        Write-Host "+ Invoke-WebRequest $dllUrl -OutFile $dllZip"
        Write-Host "+ Invoke-WebRequest $amalgamationUrl -OutFile $amalgamationZip"
        Write-Host "+ Expand-Archive $dllZip -DestinationPath $sqliteRoot -Force"
        Write-Host "+ Expand-Archive $amalgamationZip -DestinationPath $sqliteRoot -Force"
    } else {
        New-Item -ItemType Directory -Path $sqliteRoot -Force | Out-Null
        Invoke-WebRequest -Uri $dllUrl -OutFile $dllZip
        Invoke-WebRequest -Uri $amalgamationUrl -OutFile $amalgamationZip
        Expand-Archive -LiteralPath $dllZip -DestinationPath $sqliteRoot -Force
        Expand-Archive -LiteralPath $amalgamationZip -DestinationPath $sqliteRoot -Force
    }

    $vsDevCmd = Get-VsDevCmdPath
    if (-not $vsDevCmd) {
        Stop-Install "Visual Studio Build Tools are required to generate sqlite3.lib"
    }

    $hostArch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
    $command = '"{0}" -arch=x64 -host_arch={1} && lib /MACHINE:X64 /DEF:"{2}" /OUT:"{3}"' -f $vsDevCmd, $hostArch, $sqliteDef, $sqliteLib
    Invoke-Cmd $command

    $env:SQLITE3_LIB_DIR = $sqliteRoot
    $env:SQLITE3_INCLUDE_DIR = Join-Path $sqliteRoot "sqlite-amalgamation-$SqliteVersion"
}

function Build-Wallet {
    $features = if ($NoHardware) { "" } else { "hardware" }
    $featureText = if ([string]::IsNullOrWhiteSpace($features)) { "none" } else { $features }
    Write-Step "building RailOxide wallet release binary with features: $featureText"

    $vsDevCmd = Get-VsDevCmdPath
    if (-not $vsDevCmd) {
        Stop-Install "Visual Studio Build Tools are required to build RailOxide"
    }

    $manifestPath = Join-Path $SourceDir "Cargo.toml"
    $cargoArgs = @("+$Toolchain", "build", "--release", "--manifest-path", $manifestPath, "-p", "wallet")
    if (-not [string]::IsNullOrWhiteSpace($features)) {
        $cargoArgs += @("--features", $features)
    }
    $cargoArgs += @("--target", $Target)

    $hostArch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
    $cargoCommand = ($cargoArgs | ForEach-Object {
        if ($_ -match '\s') { '"{0}"' -f $_ } else { $_ }
    }) -join " "
    $command = '"{0}" -arch=x64 -host_arch={1} && cargo {2}' -f $vsDevCmd, $hostArch, $cargoCommand
    Invoke-Cmd $command
}

function Install-WalletFiles {
    $sqliteRoot = Join-Path $SourceDir ".deps\sqlite-x64"
    $walletExe = Join-Path $SourceDir "target\$Target\release\wallet.exe"
    $sqliteDll = Join-Path $sqliteRoot "sqlite3.dll"
    $iconSrc = Join-Path $SourceDir "bins\wallet\packaging\icons\windows\RailOxide.ico"

    Write-Step "installing RailOxide to $InstallDir"
    if ($DryRun) {
        Write-Host "+ New-Item -ItemType Directory -Force $InstallDir"
        Write-Host "+ Copy-Item $walletExe $InstallDir\wallet.exe"
        Write-Host "+ Copy-Item $sqliteDll $InstallDir\sqlite3.dll"
        Write-Host "+ Copy-Item $iconSrc $InstallDir\RailOxide.ico"
    } else {
        New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
        Copy-Item -LiteralPath $walletExe -Destination (Join-Path $InstallDir "wallet.exe") -Force
        Copy-Item -LiteralPath $sqliteDll -Destination (Join-Path $InstallDir "sqlite3.dll") -Force
        if (Test-Path -LiteralPath $iconSrc) {
            Copy-Item -LiteralPath $iconSrc -Destination (Join-Path $InstallDir "RailOxide.ico") -Force
        }
    }
}

function Install-Shortcut {
    if ($NoShortcut) {
        return
    }

    $startMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $shortcutPath = Join-Path $startMenuDir "RailOxide.lnk"
    $walletExe = Join-Path $InstallDir "wallet.exe"
    $iconPath = Join-Path $InstallDir "RailOxide.ico"

    Write-Step "creating Start Menu shortcut"
    if ($DryRun) {
        Write-Host "+ create shortcut $shortcutPath -> $walletExe"
        return
    }

    $shell = New-Object -ComObject WScript.Shell
    New-Item -ItemType Directory -Path $startMenuDir -Force | Out-Null
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $walletExe
    $shortcut.WorkingDirectory = $InstallDir
    if (Test-Path -LiteralPath $iconPath) {
        $shortcut.IconLocation = $iconPath
    }
    $shortcut.Save()
}

function Invoke-SmokeTest {
    $walletExe = Join-Path $InstallDir "wallet.exe"
    Write-Step "verifying installed wallet command parses --help"
    Invoke-External $walletExe @("--help")
}

function Show-DryRunPlan {
    Write-Step "dry run: no commands will be executed"
    Write-Host "repository: $RepoUrl"
    Write-Host "branch: $Branch"
    Write-Host "toolchain: $Toolchain"
    Write-Host "target: $Target"
    Write-Host "install directory: $InstallDir"
    Write-Host "source checkout: $SourceDir"
    Write-Host "system dependencies: $(-not $NoDeps)"
    Write-Host "hardware support: $(-not $NoHardware)"
    Write-Host "Start Menu shortcut: $(-not $NoShortcut)"
}

function Main {
    if ($Help) {
        Show-Usage
        return
    }

    if ($env:OS -ne "Windows_NT") {
        Stop-Install "unsupported OS: this installer requires Windows"
    }

    if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        Stop-Install "LOCALAPPDATA is not set"
    }
    if ([string]::IsNullOrWhiteSpace($env:APPDATA)) {
        Stop-Install "APPDATA is not set"
    }

    if ([string]::IsNullOrWhiteSpace($InstallDir)) {
        $script:InstallDir = Join-Path $env:LOCALAPPDATA "RailOxide\bin"
    }
    if ([string]::IsNullOrWhiteSpace($SourceDir)) {
        $script:SourceDir = Join-Path $env:LOCALAPPDATA "RailOxide\src\railoxide"
    }

    if ($DryRun) {
        Show-DryRunPlan
        return
    }

    Ensure-Dependencies
    Ensure-RequiredTools
    Ensure-RustToolchain
    Ensure-SourceCheckout
    Write-Step "building source commit $(Get-SourceCommit)"
    Ensure-SqliteForLinking
    Build-Wallet
    Install-WalletFiles
    Install-Shortcut
    Invoke-SmokeTest

    Write-Step "installed RailOxide to $InstallDir"
    Write-Step "done"
}

Main

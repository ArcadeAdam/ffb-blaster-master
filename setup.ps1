# FFBBlaster MASTER - environment setup
# Right-click this file and choose "Run with PowerShell", or from a terminal:
#   powershell -ExecutionPolicy Bypass -File .\setup.ps1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $MyInvocation.MyCommand.Path

function Step($msg) { Write-Host "`n=== $msg" -ForegroundColor Magenta }

Step "Checking for Rust"
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    Write-Host (cargo --version) -ForegroundColor Cyan
} else {
    Write-Host "Rust is not installed." -ForegroundColor Yellow
    if (Get-Command winget -ErrorAction SilentlyContinue) {
        Write-Host "Installing via winget. Accept any prompts for the Visual Studio C++ build tools."
        winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
    } else {
        Write-Host "Install Rust from https://rustup.rs and choose the default MSVC toolchain."
    }
    Write-Host "`nClose this window, open a new terminal, and run setup.ps1 again." -ForegroundColor Yellow
    Write-Host "(Rust only lands on your PATH after a restart.)" -ForegroundColor Yellow
    Read-Host "Press Enter to exit"
    exit
}

Step "Checking for the Tauri CLI"
if (Get-Command cargo-tauri -ErrorAction SilentlyContinue) {
    Write-Host (cargo tauri --version) -ForegroundColor Cyan
} else {
    Write-Host "Installing (this takes a few minutes)..."
    cargo install tauri-cli --version "^2.0"
}

Step "Running the INI editor tests"
Push-Location (Join-Path $root "src-tauri")
cargo test
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    Write-Host "`nTests failed. Do not run the app until these pass - this is the code that writes your configs." -ForegroundColor Red
    Read-Host "Press Enter to exit"
    exit 1
}

Step "Building and launching FFBBlaster MASTER"
Write-Host "First build compiles the whole dependency tree - expect several minutes." -ForegroundColor Yellow
cargo tauri dev
Pop-Location

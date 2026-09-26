# Builds reforge.dll with the MSVC toolchain and refreshes dist/.
# Needs: Rust (rustup, stable-x86_64-pc-windows-msvc) and Visual Studio Build Tools.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\build.ps1 [-Test]
param([switch]$Test)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location (Join-Path $root "native")
try {
    cargo build --release --locked --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
    $dll = Join-Path $root "native\target\x86_64-pc-windows-msvc\release\reforge.dll"
    if ($Test) {
        & (Join-Path $root "native\target\x86_64-pc-windows-msvc\release\reforge-harness.exe") $dll
        if ($LASTEXITCODE -ne 0) { throw "harness failed" }
    }
    python (Join-Path $root "scripts\make_dist.py") $dll
    if ($LASTEXITCODE -ne 0) { throw "make_dist failed" }
} finally {
    Pop-Location
}

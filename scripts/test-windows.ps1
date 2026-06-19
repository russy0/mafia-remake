$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "common-windows.ps1")
$BuildRoot = New-MafiaAsciiBuildRoot -RepoRoot $RepoRoot
$BuildRepoRoot = $BuildRoot.Root
$RustRoot = Get-MafiaWindowsRustRoot -RepoRoot $RepoRoot
$CargoHome = Join-Path $RustRoot ".cargo"
$Rustup = Join-Path $CargoHome "bin\rustup.exe"
$Toolchain = "stable-x86_64-pc-windows-gnu"

try {
if (!(Test-Path $Rustup)) {
    throw "Repo-local Rust missing. Run scripts\bootstrap-windows-rust.ps1 first."
}

Set-MafiaBuildEnvironment -RepoRoot $BuildRepoRoot -RustRoot $RustRoot

Push-Location $BuildRepoRoot
try {
    Invoke-MafiaNative $Rustup run $Toolchain cargo test
} finally {
    Pop-Location
}
} finally {
    Remove-MafiaAsciiBuildRoot -BuildRoot $BuildRoot
}

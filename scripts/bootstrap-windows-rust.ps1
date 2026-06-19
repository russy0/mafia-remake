$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "common-windows.ps1")
$BuildRoot = New-MafiaAsciiBuildRoot -RepoRoot $RepoRoot

try {
    $BuildRepoRoot = $BuildRoot.Root
    $RustRoot = Get-MafiaWindowsRustRoot -RepoRoot $RepoRoot
    $CargoHome = Join-Path $RustRoot ".cargo"
    $RustupHome = Join-Path $RustRoot ".rustup"
    $RustupInit = Join-Path $RustRoot "rustup-init.exe"
    $MingwLib = Join-Path $BuildRepoRoot ".mingw\lib"

    New-Item -ItemType Directory -Force -Path $RustRoot, $CargoHome, $RustupHome | Out-Null

    if (!(Test-Path (Join-Path $CargoHome "bin\rustup.exe"))) {
        Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $RustupInit
    }

    Set-MafiaBuildEnvironment -RepoRoot $BuildRepoRoot -RustRoot $RustRoot -QuietMissingSccache

    if (!(Test-Path (Join-Path $CargoHome "bin\rustup.exe"))) {
        Invoke-MafiaNative $RustupInit -y --profile minimal --default-host x86_64-pc-windows-gnu --default-toolchain stable-x86_64-pc-windows-gnu --no-modify-path
    }

    Invoke-MafiaNative rustup set default-host x86_64-pc-windows-gnu
    Invoke-MafiaNative rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
    Invoke-MafiaNative rustup default stable-x86_64-pc-windows-gnu
    Invoke-MafiaNative rustup target add x86_64-pc-windows-gnullvm --toolchain stable-x86_64-pc-windows-gnu
    Invoke-MafiaNative rustup component add clippy rustfmt llvm-tools --toolchain stable-x86_64-pc-windows-gnu

    New-Item -ItemType Directory -Force -Path $MingwLib | Out-Null
    $MsysLib = "C:\msys64\ucrt64\lib"
    if (Test-Path $MsysLib) {
        Copy-Item -Path (Join-Path $MsysLib "*.a") -Destination $MingwLib -Force
        Copy-Item -Path (Join-Path $MsysLib "*.o") -Destination $MingwLib -Force
    } else {
        Write-Warning "C:\msys64\ucrt64\lib not found. Install MSYS2 UCRT64 or copy MinGW import libraries into .mingw\lib."
    }

    $GccRoot = "C:\msys64\ucrt64\lib\gcc\x86_64-w64-mingw32"
    $GccLib = Get-ChildItem -Path $GccRoot -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
    if ($GccLib) {
        Copy-Item -Path (Join-Path $GccLib.FullName "libgcc*.a") -Destination $MingwLib -Force
        Copy-Item -Path (Join-Path $GccLib.FullName "crt*.o") -Destination $MingwLib -Force
        $GccEh = Join-Path $GccLib.FullName "libgcc_eh.a"
        if (Test-Path $GccEh) {
            Copy-Item -LiteralPath $GccEh -Destination (Join-Path $MingwLib "libunwind.a") -Force
        }
    } else {
        $GnuLlvmLib = Join-Path $RustupHome "toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnullvm\lib"
        $Unwind = Get-ChildItem -Path $GnuLlvmLib -Filter "libunwind-*.rlib" -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($Unwind) {
            Copy-Item -LiteralPath $Unwind.FullName -Destination (Join-Path $MingwLib "libunwind.a") -Force
        }
    }

    if ($env:MAFIA_INSTALL_SCCACHE -eq "1") {
        Install-MafiaSccache -Toolchain "stable-x86_64-pc-windows-gnu"
    } else {
        Write-Warning "Skipping sccache install. Set MAFIA_INSTALL_SCCACHE=1 to install it."
    }
    Invoke-MafiaNative rustup run stable-x86_64-pc-windows-gnu rustc "-V"
    Invoke-MafiaNative rustup run stable-x86_64-pc-windows-gnu cargo "-V"
} finally {
    Remove-MafiaAsciiBuildRoot -BuildRoot $BuildRoot
}

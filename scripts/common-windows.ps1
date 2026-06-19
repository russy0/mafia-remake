function Invoke-MafiaNative {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [Parameter(ValueFromRemainingArguments = $true)]
        [string[]]$ArgumentList
    )

    & $FilePath @ArgumentList
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE."
    }
}

function Test-MafiaAsciiPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    for ($i = 0; $i -lt $Path.Length; $i++) {
        if ([int][char]$Path[$i] -gt 127) {
            return $false
        }
    }
    return $true
}

function New-MafiaAsciiBuildRoot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $ResolvedRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
    if (Test-MafiaAsciiPath -Path $ResolvedRoot) {
        return [pscustomobject]@{
            Root = $ResolvedRoot
            Drive = $null
            Created = $false
        }
    }

    foreach ($Letter in @("M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W")) {
        $Drive = "${Letter}:"
        if ((Get-PSDrive -Name $Letter -ErrorAction SilentlyContinue) -or (Test-Path "${Drive}\")) {
            continue
        }

        & subst $Drive $ResolvedRoot
        if ($LASTEXITCODE -eq 0) {
            return [pscustomobject]@{
                Root = "${Drive}\"
                Drive = $Drive
                Created = $true
            }
        }
    }

    throw "Could not create an ASCII build drive for repo path: $ResolvedRoot"
}

function Get-MafiaWindowsRustRoot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot
    )

    $ResolvedRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
    if (Test-MafiaAsciiPath -Path $ResolvedRoot) {
        return $ResolvedRoot
    }

    if ($env:MAFIA_WINDOWS_RUST_ROOT) {
        return $env:MAFIA_WINDOWS_RUST_ROOT
    }

    return "C:\mafia-remake-rust"
}

function Remove-MafiaAsciiBuildRoot {
    param(
        [Parameter(Mandatory = $true)]
        $BuildRoot
    )

    if ($BuildRoot.Created -and $BuildRoot.Drive) {
        & subst $BuildRoot.Drive /D | Out-Null
    }
}

function Set-MafiaBuildEnvironment {
    param(
        [Parameter(Mandatory = $true)]
        [string]$RepoRoot,
        [string]$RustRoot = $RepoRoot,
        [switch]$QuietMissingSccache
    )

    if (!(Test-MafiaAsciiPath -Path $RustRoot)) {
        throw "Windows Rust root must be ASCII for MSYS2 linker: $RustRoot"
    }

    $CargoHome = Join-Path $RustRoot ".cargo"
    $RustupHome = Join-Path $RustRoot ".rustup"
    $ToolchainRoot = Join-Path $RustupHome "toolchains\stable-x86_64-pc-windows-gnu"
    $RustLibBin = Join-Path $ToolchainRoot "lib\rustlib\x86_64-pc-windows-gnu\bin"
    $Ar = Join-Path $RustLibBin "llvm-ar.exe"
    $MsysBin = "C:\msys64\ucrt64\bin"
    $MsysGcc = Join-Path $MsysBin "gcc.exe"

    if (!(Test-Path $MsysGcc)) {
        throw "MSYS2 UCRT64 gcc missing. Install MSYS2 UCRT64 or run scripts\bootstrap-windows-rust.ps1 after installing MSYS2."
    }

    $env:CARGO_HOME = $CargoHome
    $env:RUSTUP_HOME = $RustupHome
    $env:MAFIA_WORKDIR = $RepoRoot
    $env:SCCACHE_DIR = Join-Path $RepoRoot ".sccache"
    $env:CARGO_TARGET_DIR = Join-Path $RustRoot "target"
    $env:CARGO_BUILD_JOBS = ([Environment]::ProcessorCount).ToString()
    $env:Path = "$CargoHome\bin;$MsysBin;$env:Path"
    $env:AR_x86_64_pc_windows_gnu = $Ar
    $env:CC_x86_64_pc_windows_gnu = $MsysGcc
    $env:CC = $MsysGcc
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $MsysGcc
    Remove-Item Env:RUSTFLAGS -ErrorAction SilentlyContinue
    Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS -ErrorAction SilentlyContinue
    Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER -ErrorAction SilentlyContinue
    Remove-Item Env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_RUSTFLAGS -ErrorAction SilentlyContinue

    $RepoSccache = Join-Path $CargoHome "bin\sccache.exe"
    $PathSccache = Get-Command sccache -ErrorAction SilentlyContinue
    if (Test-Path $RepoSccache) {
        $env:RUSTC_WRAPPER = $RepoSccache
    } elseif ($PathSccache) {
        $env:RUSTC_WRAPPER = $PathSccache.Source
    } else {
        Remove-Item Env:RUSTC_WRAPPER -ErrorAction SilentlyContinue
        if (!$QuietMissingSccache) {
            Write-Warning "sccache not found; continuing without compiler cache. Run scripts\bootstrap-windows-rust.ps1 to install it."
        }
    }
}

function Install-MafiaSccache {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Toolchain
    )

    if (Get-Command sccache -ErrorAction SilentlyContinue) {
        return
    }

    Write-Host "Installing sccache..."
    try {
        & rustup run $Toolchain cargo install sccache --locked
        if ($LASTEXITCODE -ne 0) {
            Write-Warning "sccache install failed with exit code $LASTEXITCODE; continuing without compiler cache."
            return
        }
    } catch {
        Write-Warning "sccache install failed; continuing without compiler cache. $($_.Exception.Message)"
        return
    }

    $PathSccache = Get-Command sccache -ErrorAction SilentlyContinue
    if ($PathSccache) {
        $env:RUSTC_WRAPPER = $PathSccache.Source
    }
}

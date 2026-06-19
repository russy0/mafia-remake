#!/usr/bin/env bash

set_mafia_build_env() {
  local repo_root="$1"
  export CARGO_HOME="$repo_root/.cargo"
  export RUSTUP_HOME="$repo_root/.rustup"
  export MAFIA_WORKDIR="$repo_root"
  export SCCACHE_DIR="$repo_root/.sccache"
  export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(nproc 2>/dev/null || echo 1)}"
  export PATH="$CARGO_HOME/bin:$PATH"

  if should_use_mafia_sccache "$repo_root"; then
    export RUSTC_WRAPPER="$(command -v sccache)"
  else
    unset RUSTC_WRAPPER
    if is_wsl_windows_mount "$repo_root"; then
      if [ "${MAFIA_SCCACHE_WARN:-1}" = "1" ]; then
        echo "WSL Windows mount detected; disabling sccache to avoid /mnt/c target permission errors." >&2
      fi
    elif [ "${MAFIA_SCCACHE_WARN:-1}" = "1" ]; then
      echo "sccache not found; continuing without compiler cache. Run scripts/bootstrap-linux-rust.sh to install it." >&2
    fi
  fi
}

is_wsl_windows_mount() {
  case "$1" in
    /mnt/[a-zA-Z]/*) return 0 ;;
    *) return 1 ;;
  esac
}

should_use_mafia_sccache() {
  local repo_root="$1"

  if [ "${MAFIA_DISABLE_SCCACHE:-0}" = "1" ]; then
    return 1
  fi
  if is_wsl_windows_mount "$repo_root" && [ "${MAFIA_FORCE_SCCACHE:-0}" != "1" ]; then
    return 1
  fi
  command -v sccache >/dev/null 2>&1
}

install_mafia_sccache() {
  local toolchain="$1"
  local repo_root="${2:-$PWD}"

  if ! should_use_mafia_sccache "$repo_root"; then
    echo "Skipping sccache install/use for this filesystem. Set MAFIA_FORCE_SCCACHE=1 to override." >&2
    return 0
  fi

  if command -v sccache >/dev/null 2>&1; then
    return 0
  fi

  echo "Installing sccache..."
  if rustup run "$toolchain" cargo install sccache --locked; then
    if command -v sccache >/dev/null 2>&1; then
      export RUSTC_WRAPPER="$(command -v sccache)"
    fi
  else
    unset RUSTC_WRAPPER
    echo "sccache install failed; continuing without compiler cache." >&2
  fi
}

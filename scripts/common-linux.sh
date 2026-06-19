#!/usr/bin/env bash

set_mafia_build_env() {
  local repo_root="$1"
  export CARGO_HOME="$repo_root/.cargo"
  export RUSTUP_HOME="$repo_root/.rustup"
  export MAFIA_WORKDIR="$repo_root"
  export SCCACHE_DIR="$repo_root/.sccache"
  export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$(nproc 2>/dev/null || echo 1)}"
  export PATH="$CARGO_HOME/bin:$PATH"

  if command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER="$(command -v sccache)"
  else
    unset RUSTC_WRAPPER
    if [ "${MAFIA_SCCACHE_WARN:-1}" = "1" ]; then
      echo "sccache not found; continuing without compiler cache. Run scripts/bootstrap-linux-rust.sh to install it." >&2
    fi
  fi
}

install_mafia_sccache() {
  local toolchain="$1"

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

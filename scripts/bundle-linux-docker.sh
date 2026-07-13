#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# Espelha o job de release do Linux: mesma base (22.04 = glibc mais velha que
# suportamos) e mesmos formatos. Serve pra pegar quebra de bundle sem esperar a CI.
docker run --rm \
  -v "$PWD":/work \
  -v tyba-bundle-cargo:/root/.cargo/registry \
  -v tyba-bundle-target:/target \
  -e CARGO_TARGET_DIR=/target \
  -w /work \
  ubuntu:22.04 \
  bash -c '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq curl unzip build-essential file rpm xdg-utils \
      libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev >/dev/null
    command -v cargo >/dev/null || {
      curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable >/dev/null
      . "$HOME/.cargo/env"
    }
    . "$HOME/.cargo/env" 2>/dev/null || true
    command -v bun >/dev/null || {
      curl -fsSL https://bun.sh/install | bash >/dev/null
      export PATH="$HOME/.bun/bin:$PATH"
    }
    export PATH="$HOME/.bun/bin:$PATH"
    bun install --frozen-lockfile
    # A CI builda x86_64; aqui seguimos a arquitetura nativa do container (num Mac
    # ARM ela é aarch64). O que este script valida é o bundle — formatos gerados e
    # Depends declarados —, não o cross-compile.
    target=$(rustc -vV | sed -n "s/^host: //p")
    bunx tauri build --target "$target"
    echo "=== artefatos ==="
    find "/target/$target/release/bundle" -type f \
      \( -name "*.deb" -o -name "*.rpm" -o -name "*.AppImage" \) -exec ls -lh {} \;
    deb=$(find "/target/$target/release/bundle/deb" -name "*.deb" | head -1)
    echo "=== Depends do .deb ==="
    dpkg-deb -f "$deb" Depends
  '

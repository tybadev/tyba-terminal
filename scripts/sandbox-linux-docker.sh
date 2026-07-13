#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

# O bwrap monta um /proc novo dentro do namespace, e o Docker mascara caminhos sob
# /proc (kcore, sched_debug...) — com a máscara ativa o kernel recusa o mount e a
# jaula nem sobe. systempaths=unconfined remove a máscara do container de teste.
docker run --rm \
  --cap-add SYS_ADMIN \
  --security-opt seccomp=unconfined \
  --security-opt apparmor=unconfined \
  --security-opt systempaths=unconfined \
  -v "$PWD":/work \
  -v tyba-cargo-registry:/usr/local/cargo/registry \
  -v tyba-linux-target:/target \
  -e CARGO_TARGET_DIR=/target \
  -w /work/src-tauri \
  rust:1-bookworm \
  bash -c '
    set -euo pipefail
    apt-get update -qq
    apt-get install -y -qq bubblewrap python3 git \
      libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libgtk-3-dev >/dev/null
    cargo test --lib sandbox:: -- --nocapture --test-threads=4
  '

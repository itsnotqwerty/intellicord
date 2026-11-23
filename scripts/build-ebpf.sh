#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 0 ]]; then
  printf 'Usage: bash scripts/build-ebpf.sh\n' >&2
  exit 2
fi

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
toolchain="nightly-2026-10-15"
target="bpfel-unknown-none"
target_dir="${INTELLICORD_EBPF_TARGET_DIR:-$project_dir/crates/intellicord-ebpf/target}"
object="$target_dir/$target/release/intellicord-process"

for tool in cargo rustup bpf-linker; do
  command -v "$tool" >/dev/null || { printf 'Required tool unavailable: %s; see docs/process-observation.md\n' "$tool" >&2; exit 1; }
done
rustup run "$toolchain" rustc --version >&2
components="$(rustup component list --toolchain "$toolchain" --installed)"
if [[ $'\n'"$components"$'\n' != *$'\n'rust-src$'\n'* ]]; then
  printf 'Required component unavailable: rust-src for %s\n' "$toolchain" >&2
  exit 1
fi

cd "$project_dir"
rustup run "$toolchain" cargo build --locked --release \
  --manifest-path crates/intellicord-ebpf/Cargo.toml \
  --target "$target" --target-dir "$target_dir" \
  --config 'target.bpfel-unknown-none.linker="bpf-linker"' \
  -Z build-std=core >&2

cargo run --quiet --locked --features observe --bin intellicord-observe -- inspect --object "$object"
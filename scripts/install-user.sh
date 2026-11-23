#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]] || [[ $# -eq 1 && $1 != "--with-live-sensors" ]]; then
  printf 'Usage: ./scripts/install-user.sh [--with-live-sensors]\n' >&2
  exit 2
fi

live_sensors=false
if [[ $# -eq 1 ]]; then
  live_sensors=true
fi

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/intellicord"
target_dir="${CARGO_TARGET_DIR:-$project_dir/target}"
binary="$HOME/.local/bin/intellicord"
observer="$HOME/.local/bin/intellicord-observe"
sensor_dir="$data_dir/intellicord"
sensor_object="$sensor_dir/intellicord-process"
sensor_marker="$state_dir/live-sensors.managed"
desktop="$data_dir/applications/discord.desktop"

for tool in npm cargo install desktop-file-install desktop-file-validate update-desktop-database; do
  command -v "$tool" >/dev/null || { printf 'Required tool unavailable: %s\n' "$tool" >&2; exit 1; }
done
cd "$project_dir"
npm ci
npm run build
cargo_arguments=(build --release --locked)
if $live_sensors; then
  bash scripts/build-ebpf.sh >/dev/null
  cargo_arguments+=(--features observe)
fi
cargo "${cargo_arguments[@]}"
install -d "$HOME/.local/bin" "$data_dir/applications"
install -d -m 700 "$state_dir"
if [[ -f "$desktop" ]] && ! grep -q '^X-Intellicord-Managed=true$' "$desktop"; then
  if [[ -e "$state_dir/discord.desktop.previous" ]]; then
    printf 'A previous launcher backup already exists; inspect it before replacing another custom launcher.\n' >&2
    exit 1
  fi
  cp -p -- "$desktop" "$state_dir/discord.desktop.previous"
fi
temporary_binary="$(mktemp "$HOME/.local/bin/.intellicord.XXXXXXXX")"
temporary_observer=""
temporary_object=""
trap 'rm -f -- "$temporary_binary" "$temporary_observer" "$temporary_object"' EXIT
install -m 755 "$target_dir/release/intellicord" "$temporary_binary"
mv -f -- "$temporary_binary" "$binary"
if $live_sensors; then
  install -d "$sensor_dir"
  temporary_observer="$(mktemp "$HOME/.local/bin/.intellicord-observe.XXXXXXXX")"
  temporary_object="$(mktemp "$sensor_dir/.intellicord-process.XXXXXXXX")"
  install -m 755 "$target_dir/release/intellicord-observe" "$temporary_observer"
  install -m 644 "${INTELLICORD_EBPF_TARGET_DIR:-$project_dir/crates/intellicord-ebpf/target}/bpfel-unknown-none/release/intellicord-process" "$temporary_object"
  mv -f -- "$temporary_observer" "$observer"
  mv -f -- "$temporary_object" "$sensor_object"
  : > "$sensor_marker"
fi
desktop-file-install --dir="$data_dir/applications" packaging/discord.desktop
desktop-file-validate "$desktop"
update-desktop-database "$data_dir/applications"
if command -v kbuildsycoca6 >/dev/null; then kbuildsycoca6 --noincremental; fi
printf 'Installed %s\nDiscord launcher now attaches Intellicord on future launches.\n' "$binary"
if $live_sensors; then
  printf 'Installed opt-in process sensor artifacts: %s and %s\n' "$observer" "$sensor_object"
  printf 'Kernel observation remains unprivileged. After review, provision the constrained system helper separately with:\n  sudo %s/scripts/install-bpf-helper.sh\n' "$project_dir"
fi
printf 'No running Discord process was changed and no sensor was activated.\n'
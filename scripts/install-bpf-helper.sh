#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 0 ]]; then
  printf 'Usage: sudo ./scripts/install-bpf-helper.sh\n' >&2
  exit 2
fi
if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  printf 'This command must be run explicitly as root after reviewing the helper and object.\n' >&2
  exit 1
fi

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$project_dir/target}"
ebpf_target_dir="${INTELLICORD_EBPF_TARGET_DIR:-$project_dir/crates/intellicord-ebpf/target}"
helper="$target_dir/release/intellicord-bpf-helper"
collector="$target_dir/release/intellicord-observe"
object="$ebpf_target_dir/bpfel-unknown-none/release/intellicord-process"

for tool in install setcap getcap; do
  command -v "$tool" >/dev/null || { printf 'Required tool unavailable: %s\n' "$tool" >&2; exit 1; }
done
for artifact in "$helper" "$collector" "$object"; do
  [[ -f "$artifact" && ! -L "$artifact" ]] || { printf 'Required built artifact unavailable: %s\n' "$artifact" >&2; exit 1; }
done

install -d -o root -g root -m 755 /usr/libexec/intellicord /usr/lib/intellicord
install -o root -g root -m 755 "$helper" /usr/libexec/intellicord/intellicord-bpf-helper
install -o root -g root -m 755 "$collector" /usr/libexec/intellicord/intellicord-observe
install -o root -g root -m 644 "$object" /usr/lib/intellicord/intellicord-process
setcap cap_bpf,cap_perfmon,cap_dac_read_search=ep /usr/libexec/intellicord/intellicord-bpf-helper
capabilities="$(getcap /usr/libexec/intellicord/intellicord-bpf-helper)"
[[ "$capabilities" == *"cap_dac_read_search"* && "$capabilities" == *"cap_perfmon"* && "$capabilities" == *"cap_bpf"* && "$capabilities" == *"=ep"* ]] || {
  printf 'Capability verification failed: %s\n' "$capabilities" >&2
  exit 1
}
printf 'Installed constrained Intellicord BPF helper and protected artifacts. No sensor was started.\n'
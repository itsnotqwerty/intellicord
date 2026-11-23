#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 0 ]]; then
  printf 'Usage: sudo ./scripts/uninstall-bpf-helper.sh\n' >&2
  exit 2
fi
if [[ ${EUID:-$(id -u)} -ne 0 ]]; then
  printf 'This command must be run explicitly as root.\n' >&2
  exit 1
fi

rm -f -- \
  /usr/libexec/intellicord/intellicord-bpf-helper \
  /usr/libexec/intellicord/intellicord-observe \
  /usr/lib/intellicord/intellicord-process
rmdir --ignore-fail-on-non-empty /usr/libexec/intellicord /usr/lib/intellicord 2>/dev/null || true
printf 'Removed the constrained Intellicord BPF helper and protected artifacts. Running processes were not changed.\n'
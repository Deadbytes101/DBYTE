#!/bin/sh
# Read-only Linux collector. No temporary files, network, mounts or device writes.
set -eu
if ! command -v python3 >/dev/null 2>&1; then
    printf '%s\n' 'INVENTORY_DEPENDENCY_UNAVAILABLE: python3; nothing installed or changed' >&2
    exit 2
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 -B "$script_dir/machine_inventory_linux.py" "$@"

#!/bin/sh
# Native Linux entry point. Final mode rejects WSL in the Python verifier.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 -B "$script_dir/verify_v13_2_linux_host.py" "$@"

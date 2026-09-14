#!/bin/sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec python3 -B "$script_dir/verify_v13_2_linux_package.py" "$@"

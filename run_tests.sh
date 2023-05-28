#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

export RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}

exec cargo test --all-features --workspace "$@" -- --show-output

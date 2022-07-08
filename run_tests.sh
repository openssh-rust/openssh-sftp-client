#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

export RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}

for workspace in openssh-sftp-error openssh-sftp-client-lowlevel; do
    cd "$workspace"
    cargo test "$@" --all-features
    cd ..
done

exec cargo test "$@" --all-features

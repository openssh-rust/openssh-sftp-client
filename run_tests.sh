#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

git submodule update --init --depth 1 --recursive

export RUNTIME_DIR=${XDG_RUNTIME_DIR:-/tmp}

# Remove all files in /tmp in the container
mkdir -p $RUNTIME_DIR/openssh_sftp_client/
rm -rf $RUNTIME_DIR/openssh_sftp_client/*

cargo check "$@"

exec cargo test "$@" --all-features

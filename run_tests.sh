#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

git submodule update --init --depth 1 --recursive

if [ -z "$XDG_RUNTIME_DIR" ]; then
    export XDG_RUNTIME_DIR=/tmp
fi

# Remove all files in /tmp in the container
mkdir -p $XDG_RUNTIME_DIR/openssh_sftp_client/
rm -rf $XDG_RUNTIME_DIR/openssh_sftp_client/*

exec cargo test $@

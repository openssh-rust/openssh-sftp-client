#!/bin/sh -ex

git submodule update --init --depth 1 --recursive

# Remove all files in /tmp in the container
mkdir -p $XDG_RUNTIME_DIR/openssh_sftp_client/
rm -rf $XDG_RUNTIME_DIR/openssh_sftp_client/*

exec cargo test $@

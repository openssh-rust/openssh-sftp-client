#!/bin/bash -ex

cd $(dirname `realpath $0`)

# Clone all submodules
git submodule update --init --depth 1 --recursive

cd static-openssh-sftp-server
exec ./compile-static.sh

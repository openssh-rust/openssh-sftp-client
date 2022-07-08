#!/bin/bash -ex

set -euxo pipefail

openssh="$(dirname "$(realpath "$0")")/openssh-portable"

cd "$openssh" && git submodule update --init --depth 1 --recursive

rsync -aHAX --inplace --sparse "$openssh/" .

if [ ! -e configured ]; then
    autoreconf
    
    ./configure \
        --without-openssl \
        --without-zlib \
        --without-pam \
        --without-xauth

    touch configured
fi

exec make sftp-server -j "$(nproc)"

#!/bin/bash -ex

openssh="$(dirname `realpath $0`)/openssh-portable"

rsync -aHAX --inplace --open-noatime --sparse --preallocate  "$openssh/" .

if [ ! -e configured ]; then
    autoreconf
    
    ./configure \
        --without-openssl \
        --without-zlib \
        --without-pam \
        --without-xauth

    touch configured
fi

exec make sftp-server -j $(nproc)

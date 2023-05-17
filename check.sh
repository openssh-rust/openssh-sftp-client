#!/bin/bash

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

cargo fmt --all -- --check
cargo clippy --all-features --all --no-deps

export RUSTDOCFLAGS="--cfg docsrs"
exec cargo +nightly doc \
    --no-deps \
    --features openssh,tracing \
    --package openssh-sftp-client \
    --package openssh-sftp-error \
    --package openssh-sftp-client-lowlevel

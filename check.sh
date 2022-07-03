#!/bin/bash

run_check() {
    cargo fmt --all -- --check
    cargo +nightly fmt --all -- --check
    cargo clippy --all --no-deps
    cargo +nightly clippy --all --no-deps
    cargo doc --no-deps
}

set -euxo pipefail

cd "$(dirname "$(realpath "$0")")"

for workspace in openssh-sftp-error openssh-sftp-client-lowlevel; do
    cd "$workspace"
    run_check
    cd ..
done

run_check

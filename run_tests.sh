#!/bin/sh -ex

git submodule update --init --depth 1 --recursive

export RUSTFLAGS=--cfg=ci

exec cargo test $@

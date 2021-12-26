#!/bin/sh -ex

export RUSTFLAGS=--cfg=ci

exec cargo test $@

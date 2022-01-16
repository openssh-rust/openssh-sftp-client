#!/bin/sh -ex

git submodule update --init --depth 1 --recursive

exec cargo test $@

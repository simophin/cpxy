#!/bin/bash

cargo install cross

configs=(
  # x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
  # armv7-unknown-linux-gnueabihf
  # mips-unknown-linux-musl 
  # mipsel-unknown-linux-musl 
)

for ((i=0; i<${#configs[@]}; )); do
  target=${configs[i++]}
  echo Buildling $target
  cross build --package cjk-proxy --release --target=$target
done

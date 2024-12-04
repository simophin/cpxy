#!/bin/bash

cargo install cross

configs=(
  x86_64-unknown-linux-musl
  aarch64-unknown-linux-musl
  #armv7-unknown-linux-musl
  #mips-unknown-linux-musl 
  mipsel-unknown-linux-musl 
)

for ((i=0; i<${#configs[@]}; )); do
  target=${configs[i++]}
  echo Buildling $target
  rm -rf target/release
  rustup target add $target
  cross build --package cpxy --release --target=$target
done

#/bin/bash

rustup override set nightly
cargo install cross

for i in "$@"; do
  echo Buildling $i
  cross build --release --target=$i
done

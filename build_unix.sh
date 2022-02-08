#/bin/bash

cargo install cross

for i in "$@"; do
  echo Buildling $i
  cross build --package cjk-proxy --release --target=$i
done

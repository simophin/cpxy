#!/bin/bash

cargo install cross

for i in aarch64-unknown-linux-gnu x86_64-apple-darwin x86_64-unknown-linux-gnu aarch64-apple-darwin; do
  rustup target add $i
  mkdir -p bin/$i
  cross build --release --target $i && mv -v target/$i/release/proxy bin/$i/ || exit 1
done
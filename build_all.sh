#!/bin/bash

for i in x86_64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-gnu aarch64-unknown-linux-musl armv7-unknown-linux-gnueabihf armv7-unknown-linux-musleabihf; do
  rustup target add $i
  cargo build --release --target $i
  mv -v target/$i/release/proxy proxy-$i
done
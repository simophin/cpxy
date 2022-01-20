#!/bin/bash

for i in x86_64-unknown-linux-gnu; do
  rustup target add $i
  cargo build --release --target $i && mv -v target/$i/release/proxy proxy-$i || exit 1
done
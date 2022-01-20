#!/bin/bash

for i in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  rustup target add $i
  cross build --release --target $i && mv -v target/$i/release/proxy proxy-$i || exit 1
done
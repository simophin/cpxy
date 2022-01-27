#/bin/bash

rustup override set nightly
cargo install cross

for i in "#@"; do
  echo Buildling $i


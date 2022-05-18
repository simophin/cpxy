#!/bin/bash

configs=(
  armv7-linux-androideabi armeabi-v7a
  aarch64-linux-android arm64-v8a
  i686-linux-android x86
  x86_64-linux-android x86_64
)

cargo install cross

LIBS_ROOT=android/app/src/main/jniLibs

for ((i=0; i<${#configs[@]}; )); do
  target=${configs[i++]}
  android_target=${configs[i++]}
  echo "Building ${target}"
  cross build --package cjk-proxy --release --lib --target $target || exit 1
  mkdir -pv $LIBS_ROOT/$android_target
  cp -v target/$target/release/libproxy.so $LIBS_ROOT/$android_target/
done
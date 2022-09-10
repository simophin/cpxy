#!/bin/bash

if [[ $1 == x86_64*linux* ]]; then
    echo amd64
elif [[ $1 == aarch64*linux* ]]; then
    echo arm64v8
elif [[ $1 == armv7*linux* ]]; then
    echo arm32v7
elif [[ $1 == i686*linux* ]]; then
    echo i386
fi

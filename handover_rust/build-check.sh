#!/bin/bash
set -e
cd "$(dirname "$0")"
cargo build 2>&1 | tee build-output.txt
echo "EXIT_CODE=$?" >> build-output.txt

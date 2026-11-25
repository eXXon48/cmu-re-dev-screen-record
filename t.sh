#!/bin/sh
# Run a command in the rvp-build image (cmu-re-store/Containerfile) against this checkout.
# Usage: ./t.sh cargo test
export MSYS_NO_PATHCONV=1
exec podman run --rm -v "$(pwd -W 2>/dev/null || pwd)":/work -w /work \
  -v screen-record-target:/target -e CARGO_TARGET_DIR=/target \
  -v rvp-cargo-reg:/root/.cargo/registry -v rvp-rustup:/root/.rustup rvp-build "$@"

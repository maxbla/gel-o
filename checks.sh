#!/bin/bash

## Run this script before committing to ensure your changes don't break lints
## and conform to formatting. I'm considering writing a script to install this
## as a git pre-commit hook, but don't like the idea of mantaining bash scripts

# Disallow warnings. This can be overridden with an #[allow()] macro or simply
# be ignored, as this script is not run in CI.
export RUSTFLAGS="-D warnings"

set -x

cargo check --examples \
&& cargo clippy --examples \
&& cargo fmt -- --check \
&& cargo check --examples --all-features \
&& cargo clippy --examples --all-features
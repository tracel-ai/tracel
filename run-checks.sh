#!/bin/bash

# Exit immediately if a command exits with a non-zero status.
set -e

# This script runs all `burn-central` checks locally.
#
# Run `run-checks` using this command:
#
# ./run-checks.sh environment

# Run binary passing the first input parameter, who is mandatory.
# If the input parameter is missing or wrong, it will be the `run-checks`
# binary which will be responsible of arising an error.
cargo xtask run-checks $1

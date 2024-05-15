# This script runs all `tracel` checks locally.
#
# Run `run-checks` using this command:
#
# ./run-checks.ps1 environment

# Exit if any command fails
$ErrorActionPreference = "Stop"

# Run binary passing the first input parameter, who is mandatory.
# If the input parameter is missing or wrong, it will be the `run-checks`
# binary which will be responsible of arising an error.
cargo xtask run-checks $args[0]

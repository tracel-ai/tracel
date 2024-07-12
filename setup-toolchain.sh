#!/bin/bash

# Exit immediately if a command exits with a non-zero status.
set -e

# Parse args
mold_flag=false
mold_global_flag=false
cranelift_flag=false
help_flag=false

for arg in "$@"; do
    if [[ "$arg" == "-m" || "$arg" == "--mold" ]]; then
        mold_flag=true
    elif [[ "$arg" == "-mg" || "$arg" == "--mold-global" ]]; then
        mold_global_flag=true
    elif [[ "$arg" == "-c" || "$arg" == "--cranelift" ]]; then
        cranelift_flag=true
    elif [[ "$arg" == "-h" || "$arg" == "--help" ]]; then
        help_flag=true
    else
        echo "Unknown flag: $arg"
        exit 1
    fi
done

# Help message
if [[ "$help_flag" == true ]]; then
    echo "Usage: ./setup-toolchain.sh [flags]"
    echo "Flags:"
    echo "  -m  | --mold        : Install mold using apt-get."
    echo "  -mg | --mold-global : Install mold using apt-get and set it as the linker for all projects using the .cargo/config.toml file located in the current user's HOME (currently $HOME/.cargo/config.toml)."
    echo "  -c  | --cranelift   : Install cranelift in the rust nightly toolchain. Installs the nightly toolchain if it is not already installed. Updates rustup and the nightly toolchain if it is already installed."
    exit 0
fi

USER_HOME=$(getent passwd ${SUDO_USER:-$(whoami)} | cut -d: -f6)


if [[ "$mold_flag" == true || "$mold_global_flag" == true ]]; then
    # Check if apt-get is installed
    if ! command -v apt-get &> /dev/null; then
        echo "apt-get is not installed. This script is only supported on Debian-based systems."
        exit 1
    fi
fi

if [[ "$cranelift_flag" == true ]]; then
    # Check if rustup is installed
    if ! command -v rustup &> /dev/null; then
        echo "rustup is not installed. Please install rustup and try again."
        exit 1
    fi
fi

if [[ "$mold_flag" == true && "$mold_global_flag" == true ]]; then
    echo "The -m/--mold and -mg/--mold-global flags cannot be used together. Please use only one of them."
    exit 1
fi

# Install mold if the --mold flag is passed and setup mold as the global linker if the --mold-global flag is passed
# Refer to the mold github page for more info: https://github.com/rui314/mold
if [[ "$mold_flag" == true ]]; then
    echo "Installing mold..."

    # Install mold with apt-get
    sudo apt-get install mold

    # Check if mold is installed
    MOLD_PATH=$(which mold)
    if [ -z "$MOLD_PATH" ]; then
        echo "Mold should be installed but it could not be found in PATH. Please install mold and try again."
        exit 1
    fi

    echo "Make sure to add the following lines to your .cargo/config.toml file in your project to use mold as the linker for the current project:"
    echo -e "\n[target.x86_64-unknown-linux-gnu]\nlinker = \"clang\"\nrustflags = [\"-C\", \"link-arg=-fuse-ld=$MOLD_PATH\"]\n"
    echo "If you want to set mold as the linker for all projects, run this script with the -mg or --mold-global flag."
fi

# Add mold to the home cargo config file to use it as the linker for all projects
if [[ "$mold_global_flag" == true ]]; then
    echo "Installing mold..."

    # Install mold with apt-get
    sudo apt-get install mold

    # Check if mold is installed
    MOLD_PATH=$(which mold)
    if [ -z "$MOLD_PATH" ]; then
        echo "Mold should be installed but it could not be found in PATH. Please install mold and try again."
        exit 1
    fi

    # Global setup
    if [ ! -e "$USER_HOME/.cargo/config.toml" ]; then
        echo "Global .cargo/config.toml file does not exist. Creating it at $USER_HOME/.cargo/config.toml."
        sudo touch $USER_HOME/.cargo/config.toml
    fi

    if cat "$USER_HOME/.cargo/config.toml" | tr '\n' ' ' | grep -q -z "\[target.x86_64-unknown-linux-gnu\] linker = \"clang\" rustflags = \[\"-C\", \"link-arg=-fuse-ld=$MOLD_PATH\"\]"; then
        echo "Mold is already set as the linker in the global cargo config file $USER_HOME/.cargo/config.toml."
    else
        echo "Setting mold as the linker in the global cargo config file $USER_HOME/.cargo/config.toml."
        sudo sh -c "echo \"[target.x86_64-unknown-linux-gnu]\nlinker = \\\"clang\\\"\nrustflags = [\\\"-C\\\", \\\"link-arg=-fuse-ld=$MOLD_PATH\\\"]\n\" >> $USER_HOME/.cargo/config.toml"
    fi
fi

# Install cranelift in the rust nightly toolchain if the --cranelift flag is passed (requires a nightly toolchain so it will be installed if it is not already installed)
# Refer to the rust cranelift github page for more info: https://github.com/rust-lang/rustc_codegen_cranelift
# Cranelift might not give your performance improvements in all cases, so it is recommended to test buidling your project with and without cranelift to see if it improves the performance.
# As of writing this, I personally see a ~50% increase in build time when using cranelift, making it not worth it for me.
if [[ "$cranelift_flag" == true ]]; then
    echo "Installing cranelift in the rust nightly toolchain..."

    # Update rustup
    rustup update

    # Install nightly toolchain with rustup
    if rustup run nightly rustc --version; then
        rustup update nightly
    else
        rustup toolchain install nightly
    fi
    
    rustup component add rustc-codegen-cranelift-preview --toolchain nightly

    echo "To use the cranelift backend, refer to the github page at: https://github.com/rust-lang/rustc_codegen_cranelift?tab=readme-ov-file#download-using-rustup"
fi

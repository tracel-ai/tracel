# Tracel

## Development

### Prerequisites

- **rust**: Used by the backend and tooling. [Install rust][1].

### Before opening a pull request

To run all the checks before opening a pull request, execute the following command:

```sh
cargo xtask pull-request-checks
```

**Pro-tip:** create an alias in your shell of choice to map `cargo xtask` to something easy to type like `cx`.

For bash:

```bash
nano ~/.bashrc

# add this to the file
alias cx='cargo xtask'

# save and source the file or restart the shell session
source ~/.bashrc
```

For fish:

```fish
nano ~/.config/fish/config.fish


# add this to the file
alias cx='cargo xtask'

# save and source the file or restart the shell session
source ~/.config/fish/config.fish
```

For powershell:

```powershell
notepad $PROFILE

# add this at the end of file
function cx {
    cargo xtask $args
}

# save and quit then open a new powershell terminal
```

### Tests

To run the tests it is mandatory to use the `cargo xtask test` command as it makes sure that all
the dependencies are up and running.

The xtask commands can target different part of the monorepo:
- `crates`: the Rust crates in the cargo workspace, they are usually in `crates` directory
- `examples`: the example crates in the cargo workspace, they are in `examples` directory

To run the crates tests execute:

```sh
# run all the tests
cargo xtask test --target crates all
# run the unit tests only
cargo xtask test --target crates unit
# run the integration tests only
cargo xtask test --target crates integration
# run the documentation tests only
cargo xtask test --target crates documentation
```

To run everything:

```sh
cargo xtask test --target all all
```

[1]: https://www.rust-lang.org/tools/install

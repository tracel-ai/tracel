## To run guide-cli using the CLI, use a command of this format:

### Run local training:

```sh
cargo run --bin guide-cli -- run training --functions training --backends wgpu --configs train_configs/config.json --key <API_KEY> --project <PROJECT_ID>
```

### Run remote training:

PROJECT_VERSION is the commit hash of the project version to run, it is given when running the package command

```sh
cargo run --bin guide-cli -- package --key <API_KEY> --project <PROJECT_ID>
cargo run --bin guide-cli -- run training --functions training --backends wgpu --configs train_configs/config.json --key <API_KEY> --project <PROJECT_ID> --runner <RUNNER_GROUP_NAME> --version <PROJECT_VERSION>
```
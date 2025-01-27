# Running a project with the Heat SDK CLI:
### [Running a training locally](#run-local-training)
### [Running a training remotely](#run-remote-training)
### [Command arguments](#command-arguments)
### [Setting up a runner](#setting-up-a-runner)

<br />
<br />
<br />

## Run local training:

You can use the `run` command to run a training locally and upload training data automatically to `Heat`.\

```sh
cargo run --bin guide-cli -- run training --functions <TRAINING_FUNCTION> --backends <BURN_BACKEND> --configs <CONFIG_FILE_PATH> --key <HEAT_API_KEY> --project <PROJECT_PATH>
```

## Run remote training:

First, you need to upload the project code with the `package` command.\
This command takes your rust project and packages it into a `.crate` file which is then uploaded to `Heat`.\
The `package` command will tell you which version you just uploaded.\
To use this version in the future, you will need to specify it when running the project (either the full hash or the short version).

Then, you can run the project with the `run` command and the `--runner` flag to run it remotely on that runner group.
If you have not set up a runner yet, please follow the [**Setting up a runner**](#setting-up-a-runner) section of this file.
You can then use the project version you uploaded with the `package` command to run the project on the runner group you set up.

```sh
cargo run --bin guide-cli -- package --key <HEAT_API_KEY> --project <PROJECT_PATH>
```

```sh
cargo run --bin guide-cli -- run training --functions <TRAINING_FUNCTION> --backends <BURN_BACKEND> --configs <CONFIG_FILE_PATH> --key <HEAT_API_KEY> --project <PROJECT_PATH> --runner <RUNNER_GROUP_NAME> --version <PROJECT_VERSION>
```

## Command arguments:
TRAINING_FUNCTION: A registered training function, or space separated list of functions, in the project. To register a function, annotate it with `#[heat(training)]`.\
BURN_BACKEND: A backend, or multiple backends, supported by Burn on which you want to run the training. See [the heat-sdk-cli file](https://github.com/tracel-ai/tracel/blob/main/crates/heat-sdk-cli/src/generation/crate_gen/backend.rs) for a list of supported backends.\
CONFIG_FILE_PATH: Path(s) to the configuration file(s) for the training (relative to the crate root).\
HEAT_API_KEY: Your Heat API key. To create an API key, go to your settings page on the [Heat](https://heat.tracel.ai/) website.\
PROJECT_PATH: The identifier for the project you want to run. A project path is composed of your Heat username and the project name, separated by a slash. Note that the name is case-insensitive. Ex: `test/Default-Project.\
RUNNER_GROUP_NAME: The name of the runner group you want to run the project on. See [**Setting up a runner**](#setting-up-a-runner) for more information.\
PROJECT_VERSION: The commit hash of the project version to run. This is given when running the package command. You can also use the commit hash of a specific commit you have uploaded to Heat to run that version. You can also use the short version of the hash.

## Setting up a runner:
Two steps are required to set up a runner:

1. Create and register a runner on the `Heat` website.
    - Go to the [Heat](https://heat.tracel.ai/) website and log in.
    - Go to your `Runners` page.
    - Click on the "New runner" button and follow the instructions.
    - (Optional) On the last page, you will have the opportunity to directly assign the runner to a project by creating a runner group with same name as the runner itself in the selected project. You can also do it manually in the next step if you want more options.

2. Add the runner to a runner group in the project you want to run.
    - Go to the project page.
    - Go to the `Jobs` page.
    - Go to the `Runner Groups` tab.
    - If you already have a runner group and want to add the newly created runner to it, click on the runner group and add it by selecting the runner and the API key it should use from the dropdowns and then clicking `Assign`.
    - If you don't have a runner group yet (or do not want to add it to an existing group), click on the `Create group` button and choose a name for it. Then add the runner to the group as described above.

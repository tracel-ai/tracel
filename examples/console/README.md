# Console

A terminal tour of the Tracel console SDK, reached through the `tracel` facade crate
(`tracel::console`). Commands are grouped by noun the way `git`/`gh` are —
`console <noun> <verb>`:

- `console auth login|logout|whoami` — device authorization via `DeviceLogin::start` and
  `wait_for_approval`, session storage, and `Console::me`.
- `console org list` — `Console::organizations`.
- `console project list|show` — `Console::projects_of` and `ProjectHandle::get`.
- `console model list|show|download` — `Models::get`/`list`/`list_versions`, and
  `Models::download` streaming a version to disk with per-file progress bars through a real
  [`TransferObserver`].

`clap` handles arguments, `keyring` stores the session, `cliclack` renders the login flow and the
interactive namespace/project/model/version pickers, `terminal-link` makes the verification URL
clickable, `webbrowser` opens it on Enter, `indicatif` drives download progress bars, `ctrlc`
lets a download be cancelled mid-transfer, and `comfy-table` renders results.

The example targets a development console by default. Select the hosted console with
`--environment production` or `TRACEL_ENV=production`.

## Run

Sign in by opening the displayed URL and approving the device code:

```sh
cargo run -p console-example --example console -- auth login
```

The [`keyring`](https://crates.io/crates/keyring) crate saves the approved session in the operating
system's credential store, so the other commands can reuse it without application-specific file
or permissions code. `TRACEL_API_KEY` and `TRACEL_SESSION_TOKEN` override the saved session.
`auth logout` clears the saved session for the selected environment, and `auth whoami` displays
the signed-in user:

```sh
cargo run -p console-example --example console -- auth logout
cargo run -p console-example --example console -- auth whoami
```

Browse organizations and projects. Every namespace/project/model/version argument below is
optional — leave it out and the command prompts with a live-fetched `cliclack` menu instead:

```sh
cargo run -p console-example --example console -- org list
cargo run -p console-example --example console -- project list [namespace]
cargo run -p console-example --example console -- project show [namespace] [project]
```

List a project's models and their versions, or inspect one model:

```sh
cargo run -p console-example --example console -- model list [namespace] [project]
cargo run -p console-example --example console -- model show [namespace] [project] [model]
```

Download a published version, with a progress bar per file (Ctrl+C cancels cleanly):

```sh
cargo run -p console-example --example console -- model download [namespace] [project] [model] [version] --out ./local-dir
```

The model argument can also come from `TRACEL_MODEL`.

# common

Shared setup for the examples.

`context()` builds the SDK `Context` and picks the backend from the `TRACEL_CONNECTION`
environment variable, so an example runs offline or against the cloud without a code change:

```sh
cargo run ...                          # offline (default), records to ./runs
TRACEL_CONNECTION=cloud cargo run ...  # ships to the console
```

`cloud` needs credentials: run `tracel login` or set `TRACEL_API_KEY` first.

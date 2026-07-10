# basics

Minimal, runnable examples of the Tracel framework, using **toy capabilities shaped like real
ones** (`src/lib.rs`) so they double as templates:

- **inference** — `WordTokenizer`, a streaming inference that splits a prompt into tokens and
  streams them back.
- **experiment** — a stand-in training loop that tracks nested activities (with progress and
  cancellation) and logs metrics, exactly as a real one would. Only the per-step math is fake.

Everything runs **offline** — no credentials, telemetry recorded locally. Switch
`Connection::Offline` to `Connection::Cloud` in any example to ship to the dashboard. For the same
experiment tracking wired automatically through the Burn `train` integration, see the
[`mnist`](../mnist) example.

## Examples

| Example        | What it shows |
|----------------|---------------|
| `infer`        | Programmatic streaming inference: prompts fed over time, tokens streamed back. |
| `train`        | Programmatic experiment: nested activity tracking, metrics, cancellation. |
| `cli`          | One CLI exposing **both** jobs (`Cli::register` is uniform across capabilities). |
| `serve`        | One HTTP server exposing **both** jobs — streaming SSE for inference, fire-and-forget for training. |
| `infer-client` | Streaming HTTP client for `serve` (prompts sent over time on a chunked body). |

## Run

```sh
cargo run -p basics --example infer
cargo run -p basics --example train

# CLI: select a job by name, pass its JSON config
cargo run -p basics --example cli -- wordtok '{"text":"hello streaming world"}'
cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'

# HTTP: start the server, then hit it
cargo run -p basics --example serve
curl -N -X POST localhost:3000/wordtok      -d '{"text":"hello streaming world"}'
curl    -X POST localhost:3000/toy-training -d '{"epochs":2,"batches_per_epoch":4}'
# true streaming request (prompts over time):
cargo run -p basics --example infer-client
```

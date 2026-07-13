# basics

Small runnable examples of the Tracel framework, using toy capabilities:

- `WordTokenizer`, a streaming inference that splits a prompt into tokens.
- a stand-in training loop that tracks activities, logs metrics, and handles cancellation.

They run offline by default, so no credentials are needed. Each example gets its `Context` from
the shared [`common`](../common) crate, which chooses the backend from `TRACEL_CONNECTION`:

```sh
cargo run -p basics --example train                          # offline (default)
TRACEL_CONNECTION=cloud cargo run -p basics --example train  # ships to the console
```

The `mnist` example shows the same experiment tracking driven from the Burn `train` integration.

## Examples

| Example | Shows |
| --- | --- |
| `infer` | Streaming inference: prompts fed over time, tokens streamed back. |
| `train` | An experiment run: activity tracking, metrics, cancellation. |
| `cli` | A CLI serving both jobs. |
| `serve` | An HTTP server serving both jobs (SSE for inference, fire-and-forget for training). |
| `infer-client` | Streaming HTTP client for `serve`. |

## Run

```sh
cargo run -p basics --example infer
cargo run -p basics --example train

cargo run -p basics --example cli -- wordtok '{"text":"hello streaming world"}'
cargo run -p basics --example cli -- toy-training '{"epochs":2,"batches_per_epoch":4}'

cargo run -p basics --example serve
curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'
curl -X POST localhost:3000/toy-training -d '{"epochs":2,"batches_per_epoch":4}'
cargo run -p basics --example infer-client
```

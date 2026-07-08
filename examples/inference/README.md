# inference-example

Small, runnable examples of the Tracel streaming inference API. The inference (`WordTokenizer` in
`src/lib.rs`) splits each prompt into whitespace tokens and streams them back one at a time, with an
optional per-token delay so streaming is observable.

All examples use an **offline** connection, so no credentials are needed; per-request telemetry is
recorded locally (stubbed).

## Examples

| Example | What it shows |
|---|---|
| `manual` | Simplest path: one prompt in, tokens streamed out. |
| `streaming` | **Streaming input *and* output, in-process.** A producer feeds prompts over time; tokens come back as inputs arrive. |
| `server` | Serve the inference over HTTP; each output token is an SSE `data:` frame, ending with `done`. |
| `streaming_client` | A chunked HTTP client that streams prompts to `server` over time and prints tokens as they arrive. |

## Run

```sh
# In-process streaming input + output
cargo run -p inference-example --example streaming

# HTTP: start the server, then hit it
cargo run -p inference-example --example server
#   one-shot (buffered request, streamed response):
curl -N -X POST localhost:3000/wordtok -d '{"text":"hello streaming world"}'
#   true streaming request (prompts sent over time on a chunked body):
cargo run -p inference-example --example streaming_client
```

The `streaming` and `streaming_client` examples print millisecond timestamps so you can see that
outputs are produced as inputs arrive, not after the whole input is collected.

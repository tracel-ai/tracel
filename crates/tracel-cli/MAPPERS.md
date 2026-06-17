# Mappers

A `Mapper<I>` defines how to parse a raw CLI string into a job's input type `I`.

## Available Mappers

### JsonMapper

Deserializes the raw string as JSON. Requires `I: DeserializeOwned`.

```rust
Cli::new()
    .register_exp(my_job, JsonMapper::new())
    .run()
    .unwrap();
```

```bash
my-app experiment my_job '{"lr": 0.01, "epochs": 5}'
```

### ClapMapper

Parses the raw string as CLI arguments using clap. Requires `I: clap::Parser`.
Fields with `#[arg(default_value_t = ...)]` can be omitted by the caller.

```rust
Cli::new()
    .register_exp(my_job, ClapMapper::new())
    .run()
    .unwrap();
```

```bash
my-app experiment my_job -- --lr 0.01 --epochs 5
my-app experiment my_job -- --lr 0.01            # epochs uses default
```

### PresetMapper

Maps named aliases to predefined config objects. Requires `I: Clone`.

```rust
let mapper = PresetMapper::new()
    .preset("small", MyConfig { lr: 0.01, epochs: 5 })
    .preset("default", MyConfig { lr: 0.001, epochs: 100 });

Cli::new()
    .register_exp(my_job, mapper)
    .run()
    .unwrap();
```

```bash
my-app experiment my_job small
```

## Public API

All mappers are re-exported from the crate root:

```rust
use tracel_cli::{Cli, JsonMapper, ClapMapper, PresetMapper, Mapper};
```

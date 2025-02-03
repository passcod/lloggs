# lloggs

Logging configuration for [clap](https://docs.rs/clap) applications.

This library provides a common set of flags for controlling logging in a CLI application, and
a default implementation for configuring logging based on those flags using non-blocking
[tracing-subscriber](https://docs.rs/tracing-subscriber) when the `tracing` feature is enabled
(which is the default).

It also supports configuring logging before parsing arguments, to allow logging to be set up
using environment variables such as `RUST_LOG` or `DEBUG_INVOCATION`, respects the `NO_COLOR`
environment variable (<https://no-color.org>), and adjusts defaults when it detects systemd.

- **[API documentation](https://docs.rs/lloggs)**.
- Licensed under [Apache 2.0](./LICENSE-APACHE) or [MIT](./LICENSE-MIT).

# Example

```rust
use lloggs::{LoggingArgs, PreArgs};
use clap::Parser;

#[derive(Debug, Parser)]
struct Args {
    #[command(flatten)]
    logging: LoggingArgs,

    // Your other arguments here
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut _guard = PreArgs::parse().setup()?;
    let args = Args::parse();
    if _guard.is_none() {
        _guard = Some(args.logging.setup(|v| match v {
            0 => "info",
            1 => "debug",
            _ => "trace",
        })?);
    }

    // Your application logic here

    Ok(())
}
```

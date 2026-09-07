# Toolbox

Small personal tools, each in its own directory under `tools/`.

## Installation

Currently, installation is only supported from source using Rust/Cargo:

```sh
cargo install --git https://github.com/samlaf/toolbox --locked fleet
```

## Tools

- [fleet](tools/fleet/README.md): workspace-based power controls for existing
  Lima and Azure dev machines. Start with `cargo run -p fleet -- --help`.

Rust tools share a Cargo workspace and lockfile at the repository root.

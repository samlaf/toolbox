# fleet

Power controls for existing Lima and Azure dev machines, grouped by workspace.

## Setup

Requires Rust/Cargo and the provider CLI (`limactl` or `az`). From the repo root:

```sh
cargo install --path tools/fleet --locked
fleet ls # Creates an empty config on first use; `fleet list` also works.
fleet import lima # Register existing Lima instances, then run `fleet ls` again.
fleet check
```

Commands create a missing config and its parent directories, preserving existing
files. See [the example config](workspaces.example.toml) to register machines.
Use `--config PATH` to override
`$XDG_CONFIG_HOME/fleet/workspaces.toml` (defaults to `~/.config/fleet/workspaces.toml`).
For Azure, run `az login`; set `machine.subscription` to pin a subscription,
otherwise Fleet uses the active Azure CLI subscription.

`import lima` uses instance names as workspace names and skips registered machines.
It preserves existing settings/comments; name collisions use `lima-<name>` (then
numeric suffixes). Preview with `fleet --dry-run import lima`. Add repo paths or
Azure machines by editing the config; import currently supports Lima only.

## Usage

```sh
fleet --dry-run up local     # Preview an action without executing it
fleet up local              # Start and print connection info
fleet connect local         # Open a shell
fleet exec local -- uname -a # Run a command and return its exit code
fleet down local            # Stop Lima / deallocate Azure; never delete
```

Omit the workspace inside a configured repo or subdirectory. Repo paths only
select a workspace; Fleet does not mount, clone, or sync code yet.

Lima uses `limactl shell`, starting in `/`. Azure needs `ssh = "user@host"`
(or an SSH config alias) or `tailscale = "node"`; `ssh` takes precedence.
Use SSH config for keys, ports, and proxies. `up` waits for the provider command,
not SSH readiness; `connect` and `exec` do not start machines.

`ls` (alias `list`) shows live power states and your static hourly rates.
`check` probes Lima, Azure login, and SSH; a missing backend returns nonzero but
does not prevent using the other one. `ls` and `check` still query with `--dry-run`.

## Development

```sh
cargo test -p fleet
cargo clippy -p fleet --all-targets -- -D warnings
cargo fmt --all -- --check
```

[Design and future features](design.md).

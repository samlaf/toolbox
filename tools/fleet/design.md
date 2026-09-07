# Fleet design

Fleet maps projects to existing dev machines and provides one view of their
power state. The [README](README.md) describes what works now; the feature notes
below are future work, not supported configuration.

## Boundaries

Fleet owns workspace identity, power state, and eventually repo/connection
wiring. Keep config as pointers: machine IDs, repo paths, environment references,
and hostnames. Never grow package lists or parse another tool's definitions.

- Pulumi and Lima own machine creation and destruction. `down` never deletes.
- Nix, mise, and cloud-init own the guest environment.
- Tailscale/SSH own network access and authentication.
- Fleet stays imperative: no host daemon, reconciliation, or job scheduling.

Only Azure and Lima are in scope for now. Lima identity is an existing instance
name; a template reference must not implicitly introduce provisioning.

## Implementation shape

Keep config/selection, provider operations, subprocess handling, and UI separate
(`config.rs`, `backend.rs`, `command.rs`, and `main.rs`). Keep the backend interface
small; add connection-target and repo-wiring methods only when needed. A future
TUI should call the same core as the CLI.

Use provider CLIs and deserialize JSON for status. This keeps dependencies small,
uses existing login sessions, and leaves users with reproducible commands for
debugging. SDKs can also use ambient credentials; reconsider them only if measured
CLI latency warrants it. Keep independent status calls concurrent.

## Next features

### Repo wiring and environments

Support multiple repos per workspace, with an explicit operation per repo:

```toml
# Proposed fields; not accepted by the current parser.
repos = [
  { path = "~/src/project", mount = "/work/project" },
  { path = "~/src/infra", mount = "/work/infra" },
]
env = { nix_flake = "~/src/infra#agent-shell" }
```

Local repos use bidirectional host mounts; remote repos use `clone` destinations
instead of mounts over the network. Preserve duplicate-repo validation. Decide
how to apply Lima mounts without taking ownership of VM creation, how a host repo
maps to a remote clone URL, and how repeated wiring handles existing checkouts.
Environment activation should invoke the referenced tool, without interpreting
its contents. Define how host environment references resolve inside the guest.

### Connection readiness

Extend `up` to wait for reachability before wiring repos and printing connection
info. Add bounded SSH/Tailscale readiness checks and a Tailscale preflight to
`check`. Use stable node names for SSH and a possible VS Code Remote-SSH option,
without generating new IP-based configuration on every start.

### Pulumi identity sync

Add `fleet sync` to refresh machine identity from stack outputs. The intended
setup uses a self-managed Azure Blob backend and an explicit checkout:

```sh
PULUMI_BACKEND_URL=azblob://my-state-container \
pulumi --cwd ~/src/infra/gpu-boxes -s dev stack output --json
```

Use the stack reference appropriate to that DIY backend (original setup: `dev`).
The checkout needs its SDK dependencies and any required passphrase or Key Vault
secrets-provider access. Define the output-to-workspace mapping before building
sync. Never parse checkpoint blobs directly or persist secret outputs in config.

Sync runs when machines are added or rebuilt. Keep `up` and `ls` independent of
Pulumi: TOML remains the durable cache of identity, editable by hand.

### Resource visibility

Add reliable running-since/uptime and local allocated RAM to `ls`. Combine those
with the existing static hourly rate to expose forgotten machines; avoid billing
API integrations. Keep configured rates distinct from estimated accrued cost.

### Later

- Idle shutdown: consider an optional guest-side helper that checks SSH sessions
  and running jobs, with a configurable timeout (initial idea: 45 minutes). Define
  the Azure deallocation path; guest OS shutdown alone is not Fleet's `down`.
  Keep this separate from the imperative host CLI.
- TUI: use Rust/Ratatui after the CLI stabilizes, reusing the backend core.
- Additional providers only as needed. If ephemeral sandbox tools such as Gondolin
  become useful in the same view, consider read-only session listing first;
  snapshot/session management has different semantics from persistent VM power.

## Related tools to watch

Revisit these when planning features, especially SkyPilot. Keep links and concrete
ideas here as the tools evolve.

### SkyPilot

[Repository](https://github.com/skypilot-org/skypilot) ·
[Releases](https://github.com/skypilot-org/skypilot/releases) ·
[Blog](https://skypilot.ai/blog)

Primary reference for unified compute management, idle autostop, and remote
development workflows. Check its releases and blog before designing related Fleet
features; adapt useful interactions within Fleet's existing-machine scope.

The [Agent Sessions post](https://skypilot.ai/blog/agent-sessions) (September 1,
2026) suggests several ideas worth exploring:

- Named persistent sessions that can detach and reconnect after laptop sleep.
- Separate git worktrees/branches per session, sharing a repo's object store.
- A view of working, idle, and approval-blocked agents, with branch/diff context.
- Preserving session context while pausing compute or changing CPU/GPU resources.

The post describes Kubernetes devspaces in limited early access. Check feature
availability and source access separately; a blog announcement does not establish
that an implementation is in the public repository. For Fleet, investigate
delegating session persistence to a guest tool before expanding lifecycle scope.

### Vagrant

[CLI reference](https://developer.hashicorp.com/vagrant/docs/cli): a useful
reference for command ergonomics, global machine status, and SSH connection info.

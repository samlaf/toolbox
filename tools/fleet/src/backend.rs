use crate::{
    command::{Invocation, quote},
    config::{Machine, Workspace},
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub trait Backend {
    fn status(&self) -> Result<String>;
    fn start(&self, dry_run: bool) -> Result<i32>;
    fn stop(&self, dry_run: bool) -> Result<i32>;
}

// The enum keeps dispatch small; a future TUI can use the same backend methods.
impl Backend for Machine {
    fn status(&self) -> Result<String> {
        match self {
            Self::Lima { name } => lima_status(
                &Invocation::new("limactl", &["list", "--json"]).output()?,
                name,
            ),
            Self::Azure { .. } => {
                let mut cmd = azure(self, "get-instance-view");
                cmd.args.extend([
                    "--query".into(),
                    "instanceView.statuses".into(),
                    "--output".into(),
                    "json".into(),
                ]);
                azure_status(&cmd.output()?)
            }
        }
    }

    fn start(&self, dry_run: bool) -> Result<i32> {
        match self {
            Self::Lima { name } => {
                // limactl start also creates unknown instances. Never do that implicitly.
                if !dry_run && self.status()? == "missing" {
                    bail!("Lima instance {name:?} does not exist; create it with limactl first");
                }
                Invocation::new("limactl", &["start", "--tty=false", name]).run(dry_run)
            }
            Self::Azure { .. } => azure(self, "start").run(dry_run),
        }
    }

    fn stop(&self, dry_run: bool) -> Result<i32> {
        match self {
            Self::Lima { name } => {
                Invocation::new("limactl", &["stop", "--tty=false", name]).run(dry_run)
            }
            Self::Azure { .. } => azure(self, "deallocate").run(dry_run),
        }
    }
}

fn azure(machine: &Machine, action: &str) -> Invocation {
    let Machine::Azure {
        name,
        resource_group,
        subscription,
    } = machine
    else {
        unreachable!()
    };
    let mut cmd = Invocation::new(
        "az",
        &[
            "vm",
            action,
            "--resource-group",
            resource_group,
            "--name",
            name,
            "--only-show-errors",
        ],
    );
    if let Some(subscription) = subscription {
        cmd.args
            .extend(["--subscription".into(), subscription.clone()]);
    }
    cmd
}

pub fn connection(ws: &Workspace, command: &[String]) -> Result<Invocation> {
    if let Some(target) = ws.ssh.as_ref().or(ws.tailscale.as_ref()) {
        let mut cmd = Invocation::new("ssh", &[target]);
        if !command.is_empty() {
            cmd.args.push(
                command
                    .iter()
                    .map(|s| quote(s))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        return Ok(cmd);
    }
    match &ws.machine {
        Machine::Lima { name } => {
            let mut cmd = Invocation::new("limactl", &["shell", "--workdir=/", "--", name]);
            cmd.args.extend_from_slice(command);
            Ok(cmd)
        }
        Machine::Azure { .. } => bail!(
            "workspace {:?} needs ssh = 'user@host' (or tailscale = 'node') to connect",
            ws.name
        ),
    }
}

#[derive(Deserialize)]
struct LimaInstance {
    name: String,
    status: String,
}

pub fn lima_names() -> Result<Vec<String>> {
    let source = Invocation::new("limactl", &["list", "--json"]).output()?;
    Ok(lima_instances(&source)?
        .into_iter()
        .map(|instance| instance.name)
        .collect())
}

fn lima_instances(source: &str) -> Result<Vec<LimaInstance>> {
    // limactl emits one JSON object per instance, rather than a JSON array.
    serde_json::Deserializer::from_str(source)
        .into_iter::<LimaInstance>()
        .collect::<Result<_, _>>()
        .context("invalid limactl list JSON")
}

fn lima_status(source: &str, name: &str) -> Result<String> {
    for instance in lima_instances(source)? {
        if instance.name == name {
            return Ok(instance.status.to_lowercase());
        }
    }
    Ok("missing".into())
}

#[derive(Deserialize)]
struct AzureStatus {
    code: String,
}

fn azure_status(source: &str) -> Result<String> {
    let statuses: Vec<AzureStatus> =
        serde_json::from_str(source).context("invalid Azure instance-view status JSON")?;
    Ok(statuses
        .iter()
        .find_map(|s| s.code.strip_prefix("PowerState/"))
        .unwrap_or("unknown")
        .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_lima_stream_and_missing_instance() {
        let json = "{\"name\":\"one\",\"status\":\"Stopped\"}\n{\"name\":\"two\",\"status\":\"Running\",\"cpus\":4}\n";
        assert_eq!(lima_status(json, "two").unwrap(), "running");
        assert_eq!(lima_status(json, "absent").unwrap(), "missing");
        assert!(lima_status("not json", "two").is_err());
    }
    #[test]
    fn azure_finds_power_state_without_assuming_order() {
        assert_eq!(
            azure_status(
                r#"[{"code":"PowerState/deallocated"},{"code":"ProvisioningState/succeeded"}]"#
            )
            .unwrap(),
            "deallocated"
        );
        assert_eq!(azure_status("[]").unwrap(), "unknown");
        assert!(azure_status("null").is_err());
    }
}

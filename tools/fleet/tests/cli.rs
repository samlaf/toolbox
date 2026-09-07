#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "fleet-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self(root);
        fixture.tool(
            "limactl",
            r#"
if [ "$1" = list ]; then
  printf '%s\n' "${FLEET_TEST_LIMA_JSON}"
fi
"#,
        );
        fixture.tool(
            "az",
            r#"
if [ "$2" = get-instance-view ]; then
  if [ "$FLEET_TEST_AZ_FAIL" = 1 ]; then
    echo 'Please run az login' >&2
    exit 1
  fi
  echo '[{"code":"ProvisioningState/succeeded"},{"code":"PowerState/deallocated"}]'
fi
"#,
        );
        fixture.tool("ssh", "exit 17\n");
        fs::write(
            fixture.0.join("config.toml"),
            r#"
[[workspace]]
name = "local"
machine = { backend = "lima", name = "dev" }
[[workspace]]
name = "cloud"
machine = { backend = "azure", name = "gpu", resource_group = "dev-rg", subscription = "sub-123" }
ssh = "sam@gpu"
"#,
        )
        .unwrap();
        fixture
    }

    fn tool(&self, name: &str, body: &str) {
        let path = self.0.join(name);
        fs::write(
            &path,
            format!("#!/bin/sh\nprintf '%s\\n' '{name}' \"$@\" >> \"$FLEET_TEST_LOG\"\n{body}"),
        )
        .unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn command(&self) -> Command {
        let mut cmd = self.default_command();
        cmd.args(["--config", self.0.join("config.toml").to_str().unwrap()]);
        cmd
    }

    fn default_command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_fleet"));
        cmd.env("PATH", &self.0)
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("settings"))
            .env("FLEET_TEST_LOG", self.0.join("calls"))
            .env(
                "FLEET_TEST_LIMA_JSON",
                r#"{"name":"dev","status":"Stopped"}"#,
            )
            .env_remove("FLEET_TEST_AZ_FAIL");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.0.join("calls")).unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn refuses_to_create_a_missing_lima_instance() {
    let f = Fixture::new();
    let out = f
        .command()
        .env("FLEET_TEST_LIMA_JSON", "")
        .args(["up", "local"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not exist"));
    assert_eq!(f.calls(), "limactl\nlist\n--json\n");
}

#[test]
fn starts_existing_lima_and_deallocates_azure_in_pinned_subscription() {
    let f = Fixture::new();
    assert!(f.run(&["up", "local"]).status.success());
    assert!(f.calls().contains("limactl\nstart\n--tty=false\ndev\n"));
    assert!(f.run(&["down", "cloud"]).status.success());
    assert!(f.calls().contains("az\nvm\ndeallocate\n--resource-group\ndev-rg\n--name\ngpu\n--only-show-errors\n--subscription\nsub-123\n"));
}

#[test]
fn dry_run_performs_no_provider_or_ssh_calls() {
    let f = Fixture::new();
    for args in [
        vec!["--dry-run", "up", "local"],
        vec!["--dry-run", "down", "cloud"],
        vec!["--dry-run", "connect", "cloud"],
        vec!["--dry-run", "exec", "local", "--", "echo", "hello"],
    ] {
        assert!(f.run(&args).status.success());
    }
    assert_eq!(f.calls(), "");
}

#[test]
fn list_keeps_healthy_rows_when_another_backend_fails() {
    let f = Fixture::new();
    let out = f
        .command()
        .env("FLEET_TEST_AZ_FAIL", "1")
        .arg("ls")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("stopped"));
    assert!(stdout.contains("error"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("az login"));
}

#[test]
fn exec_quotes_remote_arguments_and_propagates_exit_code() {
    let f = Fixture::new();
    let out = f.run(&[
        "exec",
        "cloud",
        "--",
        "printf",
        "%s",
        "hello; $(whoami)",
        "",
    ]);
    assert_eq!(out.status.code(), Some(17));
    assert_eq!(f.calls(), "ssh\nsam@gpu\nprintf %s 'hello; $(whoami)' ''\n");
}

#[test]
fn list_alias_matches_ls_and_preserves_existing_config() {
    let f = Fixture::new();
    let before = fs::read(f.0.join("config.toml")).unwrap();
    let ls = f.run(&["ls"]);
    let list = f.run(&["list"]);
    assert!(ls.status.success() && list.status.success());
    assert_eq!(ls.stdout, list.stdout);
    assert_eq!(before, fs::read(f.0.join("config.toml")).unwrap());
}

#[test]
fn every_subcommand_initializes_missing_default_config() {
    for args in [
        vec!["ls"],
        vec!["list"],
        vec!["up"],
        vec!["down", "local"],
        vec!["connect"],
        vec!["exec", "--", "uname"],
        vec!["check"],
    ] {
        let f = Fixture::new();
        let out = f.default_command().args(&args).output().unwrap();
        assert!(
            String::from_utf8_lossy(&out.stdout).contains("No machines registered."),
            "{args:?}: {out:?}"
        );
        assert_eq!(
            fs::read(f.0.join("settings/fleet/workspaces.toml")).unwrap(),
            b""
        );
        if args[0] == "check" {
            assert!(f.calls().contains("--version"));
        } else {
            assert!(f.calls().is_empty());
            assert_eq!(out.status.success(), matches!(args[0], "ls" | "list"));
        }
    }
}

#[test]
fn creates_explicit_relative_config_and_home_default() {
    let f = Fixture::new();
    let out = f
        .default_command()
        .current_dir(&f.0)
        .args(["--config", "workspaces.toml", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(fs::read(f.0.join("workspaces.toml")).unwrap(), b"");
    let out = f
        .default_command()
        .env_remove("XDG_CONFIG_HOME")
        .arg("ls")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        fs::read(f.0.join(".config/fleet/workspaces.toml")).unwrap(),
        b""
    );
}

#[test]
fn invalid_config_and_filesystem_errors_are_not_treated_as_empty() {
    let f = Fixture::new();
    fs::write(f.0.join("config.toml"), "invalid = [").unwrap();
    let out = f.run(&["ls"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid config"));
    assert_eq!(
        fs::read_to_string(f.0.join("config.toml")).unwrap(),
        "invalid = ["
    );
    let out = f
        .default_command()
        .arg("--config")
        .arg(f.0.join("config.toml/child.toml"))
        .arg("ls")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("No machines registered."));
}

#[test]
fn import_initializes_config_and_registers_all_instances() {
    let f = Fixture::new();
    let json =
        "{\"name\":\"one\",\"status\":\"Stopped\"}\n{\"name\":\"two\",\"status\":\"Running\"}";
    let out = f
        .default_command()
        .env("FLEET_TEST_LIMA_JSON", json)
        .args(["import", "lima"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let source = fs::read_to_string(f.0.join("settings/fleet/workspaces.toml")).unwrap();
    let config: toml::Value = toml::from_str(&source).unwrap();
    let workspaces = config["workspace"].as_array().unwrap();
    assert_eq!(workspaces.len(), 2);
    assert_eq!(workspaces[0]["name"].as_str(), Some("one"));
    assert_eq!(workspaces[1]["machine"]["name"].as_str(), Some("two"));
    assert_eq!(workspaces[1]["machine"]["backend"].as_str(), Some("lima"));
    assert_eq!(f.calls(), "limactl\nlist\n--json\n");
    assert!(
        f.default_command()
            .args(["--dry-run", "connect", "one"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn import_preserves_settings_skips_known_machines_and_resolves_name_collisions() {
    let f = Fixture::new();
    let path = f.0.join("config.toml");
    let before = format!(
        "# My workspaces\n{}\n# Keep this note\n",
        fs::read_to_string(&path).unwrap()
    );
    fs::write(&path, &before).unwrap();
    let json = "{\"name\":\"dev\",\"status\":\"Stopped\"}\n{\"name\":\"local\",\"status\":\"Stopped\"}\n{\"name\":\"local\",\"status\":\"Stopped\"}";
    let out = f
        .command()
        .env("FLEET_TEST_LIMA_JSON", json)
        .args(["import", "lima"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.starts_with("# My workspaces\n"));
    assert!(after.contains("# Keep this note"));
    let config: toml::Value = toml::from_str(&after).unwrap();
    let original: toml::Value = toml::from_str(&before).unwrap();
    assert_eq!(config["workspace"][0], original["workspace"][0]);
    assert_eq!(config["workspace"][1], original["workspace"][1]);
    assert_eq!(config["workspace"].as_array().unwrap().len(), 3);
    assert_eq!(config["workspace"][2]["name"].as_str(), Some("lima-local"));
    let out = f
        .command()
        .env("FLEET_TEST_LIMA_JSON", json)
        .args(["import", "lima"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("No new Lima machines"));
    assert_eq!(fs::read_to_string(path).unwrap(), after);
}

#[test]
fn import_preview_and_invalid_discovery_leave_config_untouched() {
    let f = Fixture::new();
    let path = f.0.join("config.toml");
    let before = fs::read(&path).unwrap();
    let out = f
        .command()
        .env(
            "FLEET_TEST_LIMA_JSON",
            r#"{"name":"new","status":"Stopped"}"#,
        )
        .args(["--dry-run", "import", "lima"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Would import new"));
    assert_eq!(fs::read(&path).unwrap(), before);
    for json in [
        "{\"name\":\"new\",\"status\":\"Stopped\"}\nnot json",
        r#"{"name":"--bad","status":"Stopped"}"#,
    ] {
        let out = f
            .command()
            .env("FLEET_TEST_LIMA_JSON", json)
            .args(["import", "lima"])
            .output()
            .unwrap();
        assert!(!out.status.success());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    f.tool("limactl", "echo 'provider failed' >&2\nexit 1\n");
    assert!(!f.run(&["import", "lima"]).status.success());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn import_supports_inline_workspace_arrays_and_empty_discovery() {
    for source in [
        "# Nothing yet\nworkspace = []\n",
        "workspace = [{ name = 'local', machine = { backend = 'lima', name = 'another' } }]\n",
    ] {
        let f = Fixture::new();
        let path = f.0.join("config.toml");
        fs::write(&path, source).unwrap();
        let out = f
            .command()
            .env("FLEET_TEST_LIMA_JSON", "")
            .args(["import", "lima"])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert!(f.run(&["import", "lima"]).status.success());
        assert!(f.run(&["--dry-run", "connect", "dev"]).status.success());
    }
}

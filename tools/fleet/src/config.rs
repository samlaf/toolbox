use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use std::{
    collections::HashSet,
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub workspace: Vec<Workspace>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub name: String,
    pub machine: Machine,
    #[serde(default)]
    pub repos: Vec<Repo>,
    /// SSH destination (an SSH config alias or user@host).
    pub ssh: Option<String>,
    pub tailscale: Option<String>,
    pub hourly: Option<f64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "backend", rename_all = "lowercase", deny_unknown_fields)]
pub enum Machine {
    Lima {
        name: String,
    },
    Azure {
        name: String,
        resource_group: String,
        subscription: Option<String>,
    },
}

impl Machine {
    pub fn name(&self) -> &str {
        match self {
            Self::Lima { name } | Self::Azure { name, .. } => name,
        }
    }

    pub fn backend(&self) -> &str {
        match self {
            Self::Lima { .. } => "lima",
            Self::Azure { .. } => "azure",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repo {
    pub path: PathBuf,
}

pub fn default_path() -> Result<PathBuf> {
    let base = match env::var_os("XDG_CONFIG_HOME") {
        Some(path) => PathBuf::from(path),
        None => home()?.join(".config"),
    };
    Ok(base.join("fleet/workspaces.toml"))
}

fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; use --config with an absolute path")
}

pub fn expand(path: &Path) -> Result<PathBuf> {
    if path.starts_with("~") {
        Ok(home()?.join(path.strip_prefix("~")?))
    } else {
        Ok(path.to_path_buf())
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let path = expand(path)?;
        let source = match fs::read_to_string(&path) {
            Err(err) if err.kind() == ErrorKind::NotFound => {
                if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("cannot create {}", parent.display()))?;
                }
                // Never truncate a config created by another invocation in the meantime.
                match fs::File::create_new(&path) {
                    Ok(_) => {}
                    Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
                    Err(err) => {
                        return Err(err)
                            .with_context(|| format!("cannot create {}", path.display()));
                    }
                }
                fs::read_to_string(&path)
            }
            result => result,
        }
        .with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&source).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn parse(source: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(source)?;
        let mut names = HashSet::new();
        let mut repos = HashSet::new();
        for ws in &mut config.workspace {
            ensure!(!ws.name.trim().is_empty(), "workspace name cannot be empty");
            ensure!(
                names.insert(ws.name.clone()),
                "duplicate workspace {:?}",
                ws.name
            );
            let name = ws.machine.name();
            ensure!(
                !name.is_empty()
                    && !name.starts_with('-')
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
                    && name != "."
                    && name != "..",
                "invalid machine name {name:?}"
            );
            if let Machine::Azure {
                resource_group,
                subscription,
                ..
            } = &ws.machine
            {
                ensure!(
                    !resource_group.trim().is_empty() && !resource_group.starts_with('-'),
                    "invalid Azure resource_group"
                );
                if let Some(s) = subscription {
                    ensure!(
                        !s.trim().is_empty() && !s.starts_with('-'),
                        "invalid Azure subscription"
                    );
                }
            }
            for target in [&ws.ssh, &ws.tailscale].into_iter().flatten() {
                ensure!(
                    !target.is_empty()
                        && !target.starts_with('-')
                        && !target.chars().any(|c| c.is_whitespace() || c.is_control()),
                    "invalid SSH destination {target:?}"
                );
            }
            if let Some(rate) = ws.hourly {
                ensure!(
                    rate.is_finite() && rate >= 0.0,
                    "hourly must be a finite, nonnegative number"
                );
            }
            for repo in &mut ws.repos {
                repo.path = expand(&repo.path)?;
                ensure!(
                    repo.path.is_absolute(),
                    "repo path must be absolute or start with ~/: {}",
                    repo.path.display()
                );
                // Canonicalize existing paths so symlink aliases cannot select two workspaces.
                repo.path = repo.path.canonicalize().unwrap_or_else(|_| {
                    repo.path.components().fold(PathBuf::new(), |mut p, c| {
                        if c == std::path::Component::ParentDir {
                            p.pop();
                        } else {
                            p.push(c);
                        }
                        p
                    })
                });
                ensure!(
                    repos.insert(repo.path.clone()),
                    "repo belongs to more than one entry: {}",
                    repo.path.display()
                );
            }
        }
        Ok(config)
    }

    pub fn select(&self, name: Option<&str>, cwd: &Path) -> Result<&Workspace> {
        if let Some(name) = name {
            return self
                .workspace
                .iter()
                .find(|ws| ws.name == name)
                .with_context(|| format!("unknown workspace {name:?}"));
        }
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let matches: Vec<_> = self
            .workspace
            .iter()
            .filter(|ws| ws.repos.iter().any(|repo| cwd.starts_with(&repo.path)))
            .collect();
        match matches.as_slice() {
            [ws] => Ok(ws),
            [] => bail!(
                "no workspace matches {}; pass a workspace name",
                cwd.display()
            ),
            _ => bail!(
                "multiple workspaces match {}; pass a workspace name",
                cwd.display()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const BASE: &str = "[[workspace]]\nname = 'dev'\nmachine = { backend = 'lima', name = 'dev' }\nrepos = [{ path = '/fleet-test/repo' }]";

    #[test]
    fn inference_uses_path_boundaries() {
        let config = Config::parse(BASE).unwrap();
        assert_eq!(
            config
                .select(None, Path::new("/fleet-test/repo/src"))
                .unwrap()
                .name,
            "dev"
        );
        assert!(
            config
                .select(None, Path::new("/fleet-test/repository"))
                .is_err()
        );
        assert!(config.select(Some("missing"), Path::new("/")).is_err());
    }

    #[test]
    fn rejects_duplicates_and_unsupported_fields() {
        assert!(Config::parse(&format!("{BASE}\n{BASE}")).is_err());
        assert!(
            Config::parse(&format!(
                "{BASE}\n{}",
                BASE.replace("name = 'dev'", "name = 'other'")
            ))
            .is_err()
        );
        assert!(Config::parse(&format!("{BASE}\nenv = {{ nix_flake = 'foo' }}")).is_err());
        assert!(Config::parse(&BASE.replace("backend = 'lima'", "backend = 'gcloud'")).is_err());
        assert!(Config::parse(&format!("{BASE}\nssh = '-oProxyCommand=bad'")).is_err());
    }

    #[test]
    fn nested_repos_are_ambiguous() {
        let config = Config::parse(&format!(
            "{BASE}\n{}",
            BASE.replace("name = 'dev'", "name = 'other'")
                .replace("/fleet-test/repo", "/fleet-test/repo/nested")
        ))
        .unwrap();
        assert!(
            config
                .select(None, Path::new("/fleet-test/repo/nested/src"))
                .is_err()
        );
    }
}

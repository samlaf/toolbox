use anyhow::{Context, Result, ensure};
use std::process::{Command, Stdio};

#[derive(Debug)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
}

impl Invocation {
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(&self.args)
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn output(&self) -> Result<String> {
        let output = Command::new(&self.program)
            .args(&self.args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| {
                format!(
                    "cannot run {}; is {} installed and on PATH?",
                    self.display(),
                    self.program
                )
            })?;
        ensure!(
            output.status.success(),
            "{} failed ({}): {}",
            self.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        String::from_utf8(output.stdout).context("command returned non-UTF-8 output")
    }

    pub fn run(&self, dry_run: bool) -> Result<i32> {
        eprintln!(
            "{}{}",
            if dry_run { "[dry-run] " } else { "+ " },
            self.display()
        );
        if dry_run {
            return Ok(0);
        }
        let status = Command::new(&self.program)
            .args(&self.args)
            .status()
            .with_context(|| format!("cannot run {}", self.display()))?;
        Ok(status.code().unwrap_or(1))
    }
}

// SSH sends a command string through the remote shell. Quote each argument there,
// while always passing local commands directly to Command (never sh -c).
pub fn quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
    {
        s.into()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_arguments_are_shell_quoted() {
        assert_eq!(quote(""), "''");
        assert_eq!(quote("hello; $(whoami)"), "'hello; $(whoami)'");
        assert_eq!(quote("it's"), "'it'\\''s'");
    }
}

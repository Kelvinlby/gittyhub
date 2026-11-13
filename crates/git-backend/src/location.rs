//! Where a repository lives: on this machine, or on a remote host reached over SSH.
//!
//! Remote repositories are driven by running `git` on the remote host through the system
//! `ssh` client. Keys, agents and host configuration all come from the user's own `~/.ssh`
//! setup and `ssh-agent`; GittyHub never reads or stores key material.

use std::fmt;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Location {
    Local(PathBuf),
    Ssh { host: String, path: String },
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Local(p) => write!(f, "{}", p.display()),
            Location::Ssh { host, path } => write!(f, "{host}:{path}"),
        }
    }
}

/// Quote `s` for a POSIX shell.
pub fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@,+%^".contains(c))
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Base `ssh` command with connection sharing so repeated git calls stay fast.
pub fn ssh_command(host: &str) -> Command {
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    let mut cmd = Command::new("ssh");
    cmd.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ControlMaster=auto",
        "-o",
        &format!("ControlPath={runtime}/gittyhub-ssh-%C"),
        "-o",
        "ControlPersist=120",
        host,
        "--",
    ]);
    cmd
}

impl Location {
    /// Short name for tabs: the last path component.
    pub fn name(&self) -> String {
        let p = match self {
            Location::Local(p) => p.to_string_lossy().into_owned(),
            Location::Ssh { path, .. } => path.clone(),
        };
        p.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_owned()
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Location::Ssh { .. })
    }

    /// Same host, different path.
    pub fn with_path(&self, path: &str) -> Location {
        match self {
            Location::Local(_) => Location::Local(path.into()),
            Location::Ssh { host, .. } => Location::Ssh {
                host: host.clone(),
                path: path.to_owned(),
            },
        }
    }

    /// Build a command that runs `git <args>` inside this location.
    pub fn git_command(&self, args: &[&str]) -> Command {
        self.git_command_env(&[], args)
    }

    /// Like [`Location::git_command`] with extra environment variables (which beat git config).
    pub fn git_command_env(&self, env: &[(&str, &str)], args: &[&str]) -> Command {
        match self {
            Location::Local(path) => {
                let mut cmd = Command::new("git");
                cmd.arg("-C").arg(path).args(args);
                cmd.env("GIT_TERMINAL_PROMPT", "0");
                cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
                cmd.env("LC_ALL", "C");
                cmd.envs(env.iter().copied());
                cmd
            }
            Location::Ssh { host, path } => {
                let mut cmd = ssh_command(host);
                let mut line = String::from(
                    "env GIT_TERMINAL_PROMPT=0 'GIT_SSH_COMMAND=ssh -o BatchMode=yes' LC_ALL=C",
                );
                for (k, v) in env {
                    line.push(' ');
                    line.push_str(&shell_quote(&format!("{k}={v}")));
                }
                line.push_str(" git -C ");
                line.push_str(&shell_quote(path));
                for a in args {
                    line.push(' ');
                    line.push_str(&shell_quote(a));
                }
                cmd.arg(line);
                cmd
            }
        }
    }

    /// Run git, optionally feeding `stdin`, returning the raw output.
    pub fn git_output(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<Output> {
        self.git_output_env(&[], args, stdin)
    }

    pub fn git_output_env(
        &self,
        env: &[(&str, &str)],
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<Output> {
        let mut cmd = self.git_command_env(env, args);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd.spawn().context("failed to spawn git")?;
        if let Some(input) = stdin {
            let mut pipe = child.stdin.take().context("no stdin")?;
            pipe.write_all(input)?;
        }
        Ok(child.wait_with_output()?)
    }

    /// Run git and return stdout, failing on a non-zero exit.
    pub fn git(&self, args: &[&str]) -> Result<String> {
        self.git_in(args, None)
    }

    pub fn git_in(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<String> {
        self.git_env(&[], args, stdin)
    }

    pub fn git_env(
        &self,
        env: &[(&str, &str)],
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<String> {
        let out = self.git_output_env(env, args, stdin)?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let msg = if err.trim().is_empty() {
                String::from_utf8_lossy(&out.stdout).trim().to_owned()
            } else {
                err.trim().to_owned()
            };
            bail!("git {}: {}", args.first().copied().unwrap_or(""), msg);
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run git and return stdout whatever the exit status (for `diff --no-index` and friends).
    pub fn git_lenient(&self, args: &[&str]) -> Result<String> {
        let out = self.git_output(args, None)?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run a shell snippet inside the repository directory (on the remote host for SSH).
    pub fn shell(&self, script: &str, stdin: Option<&[u8]>) -> Result<Vec<u8>> {
        let mut cmd = match self {
            Location::Local(path) => {
                let mut c = Command::new("sh");
                c.arg("-c").arg(script).current_dir(path);
                c
            }
            Location::Ssh { host, path } => {
                let mut c = ssh_command(host);
                c.arg(format!("cd {} && {script}", shell_quote(path)));
                c
            }
        };
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd.spawn().context("failed to spawn shell")?;
        if let Some(input) = stdin {
            child.stdin.take().context("no stdin")?.write_all(input)?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(out.stdout)
    }

    /// Read a work-tree file (path relative to the repository root).
    pub fn read_file(&self, rel: &str) -> Result<Vec<u8>> {
        match self {
            Location::Local(p) => Ok(std::fs::read(p.join(rel))?),
            Location::Ssh { .. } => self.shell(&format!("cat -- {}", shell_quote(rel)), None),
        }
    }

    /// Replace a work-tree file's contents.
    pub fn write_file(&self, rel: &str, data: &[u8]) -> Result<()> {
        match self {
            Location::Local(p) => Ok(std::fs::write(p.join(rel), data)?),
            Location::Ssh { .. } => self
                .shell(&format!("cat > {}", shell_quote(rel)), Some(data))
                .map(drop),
        }
    }

    /// Run git, returning whether it succeeded (for `--verify`-style probes).
    pub fn git_ok(&self, args: &[&str]) -> bool {
        self.git_output(args, None)
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("abc/def"), "abc/def");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn names() {
        let l = Location::Ssh {
            host: "box".into(),
            path: "/srv/proj/".into(),
        };
        assert_eq!(l.name(), "proj");
        assert_eq!(l.to_string(), "box:/srv/proj/");
    }
}

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

use crate::error::StackError;

#[derive(Debug, Clone)]
pub struct GitRepo {
    pub root: PathBuf,
}

impl GitRepo {
    pub fn discover(path: PathBuf) -> Result<Self> {
        let output = Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(&path)
            .output()
            .context("failed to run git rev-parse")?;
        if !output.status.success() {
            anyhow::bail!("current directory is not inside a git repository");
        }
        let root = String::from_utf8(output.stdout)?.trim().to_string();
        Ok(Self {
            root: PathBuf::from(root),
        })
    }

    pub fn is_clean(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["diff-index", "--quiet", "HEAD", "--"])
            .current_dir(&self.root)
            .output()
            .context("failed to run git diff-index")?;
        Ok(output.status.success())
    }

    pub fn ensure_clean(&self) -> Result<()> {
        if self.is_clean()? {
            Ok(())
        } else {
            Err(StackError::DirtyWorktree.into())
        }
    }

    pub fn current_branch(&self) -> Result<String> {
        self.git(&["branch", "--show-current"])
    }

    pub fn branch_exists(&self, name: &str) -> Result<bool> {
        Ok(self.run_status(&[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ])?)
    }

    pub fn ensure_branch_exists(&self, name: &str) -> Result<()> {
        if self.branch_exists(name)? {
            Ok(())
        } else {
            anyhow::bail!("branch does not exist locally: {name}")
        }
    }

    pub fn ensure_commit_exists(&self, rev: &str) -> Result<()> {
        if self.run_status(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])? {
            Ok(())
        } else {
            anyhow::bail!("commit does not exist: {rev}")
        }
    }

    pub fn rev_parse(&self, rev: &str) -> Result<String> {
        self.git(&["rev-parse", rev])
    }

    pub fn fetch(&self, remote: &str) -> Result<()> {
        self.run(&["fetch", remote])
    }

    pub fn create_branch(&self, branch: &str, start_point: &str) -> Result<()> {
        self.run(&["branch", branch, start_point])
    }

    pub fn checkout(&self, branch: &str) -> Result<()> {
        self.run(&["checkout", branch])
    }

    pub fn delete_branch(&self, branch: &str) -> Result<()> {
        self.run(&["branch", "-d", branch])
    }

    pub fn rebase_onto(&self, new_base: &str, old_base: &str, branch: &str) -> Result<()> {
        self.run(&["rebase", "--onto", new_base, old_base, branch])
    }

    pub fn rebase_continue(&self) -> Result<()> {
        self.run(&["rebase", "--continue"])
    }

    pub fn rebase_abort(&self) -> Result<()> {
        self.run(&["rebase", "--abort"])
    }

    pub fn is_rebase_in_progress(&self) -> Result<bool> {
        Ok(self.git_path("rebase-merge")?.exists() || self.git_path("rebase-apply")?.exists())
    }

    pub fn branch_tip(&self, branch: &str) -> Result<String> {
        self.rev_parse(branch)
    }

    fn git_path(&self, path: &str) -> Result<PathBuf> {
        Ok(PathBuf::from(self.git(&[
            "rev-parse",
            "--git-path",
            path,
        ])?))
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .with_context(|| format!("failed to run git {}", args.join(" ")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    fn run(&self, args: &[&str]) -> Result<()> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .with_context(|| format!("failed to run git {}", args.join(" ")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
        }
        Ok(())
    }

    fn run_status(&self, args: &[&str]) -> Result<bool> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .output()
            .with_context(|| format!("failed to run git {}", args.join(" ")))?;
        Ok(output.status.success())
    }
}

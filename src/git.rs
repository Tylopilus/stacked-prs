use std::fs;
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

    pub fn add_paths(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run(&args)
    }

    pub fn rebase_abort(&self) -> Result<()> {
        self.run(&["rebase", "--abort"])
    }

    pub fn is_rebase_in_progress(&self) -> Result<bool> {
        Ok(self.git_path("rebase-merge")?.exists() || self.git_path("rebase-apply")?.exists())
    }

    pub fn rebase_branch_and_onto(&self) -> Result<Option<(String, String)>> {
        for dir_name in ["rebase-merge", "rebase-apply"] {
            let dir = self.git_path(dir_name)?;
            if !dir.exists() {
                continue;
            }
            let head_name = fs::read_to_string(dir.join("head-name"))?
                .trim()
                .to_string();
            let branch = head_name
                .strip_prefix("refs/heads/")
                .unwrap_or(&head_name)
                .to_string();
            let onto = fs::read_to_string(dir.join("onto"))?.trim().to_string();
            return Ok(Some((branch, onto)));
        }
        Ok(None)
    }

    pub fn unmerged_paths(&self) -> Result<Vec<String>> {
        let output = self.git(&["diff", "--name-only", "--diff-filter=U"])?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    pub fn paths_with_conflict_markers(&self, paths: &[String]) -> Result<Vec<String>> {
        let mut unresolved = Vec::new();
        for path in paths {
            let full_path = self.root.join(path);
            let Ok(contents) = fs::read_to_string(&full_path) else {
                unresolved.push(path.clone());
                continue;
            };
            if contents.lines().any(|line| {
                line.starts_with("<<<<<<< ")
                    || line.starts_with("=======")
                    || line.starts_with(">>>>>>> ")
            }) {
                unresolved.push(path.clone());
            }
        }
        Ok(unresolved)
    }

    pub fn branch_tip(&self, branch: &str) -> Result<String> {
        self.rev_parse(branch)
    }

    pub fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        self.run_status(&["merge-base", "--is-ancestor", ancestor, descendant])
    }

    pub fn push(&self, remote: &str, branch: &str, force_with_lease: bool) -> Result<()> {
        if force_with_lease {
            self.run(&[
                "push",
                "--force-with-lease",
                "--set-upstream",
                remote,
                branch,
            ])
        } else {
            self.run(&["push", "--set-upstream", remote, branch])
        }
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

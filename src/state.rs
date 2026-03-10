use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::StackError;
use crate::git::GitRepo;
use crate::graph;

const STACK_DIR: &str = ".stacked-prs";
const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoState {
    pub version: u32,
    pub repo: RepoConfig,
    pub branches: Vec<ManagedBranch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoConfig {
    pub trunk: String,
    pub remote: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedBranch {
    pub name: String,
    pub parent: String,
    pub recorded_parent_tip: String,
    pub status: BranchStatus,
    pub pr: Option<PullRequestRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BranchStatus {
    Active,
    Merged,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestRef {
    pub provider: String,
    pub id: String,
    pub url: Option<String>,
    pub target_branch: Option<String>,
}

impl RepoState {
    pub fn new(trunk: String, remote: String) -> Self {
        Self {
            version: 1,
            repo: RepoConfig { trunk, remote },
            branches: Vec::new(),
        }
    }

    pub fn path_in(root: &Path) -> PathBuf {
        root.join(STACK_DIR).join(STATE_FILE)
    }

    pub fn load(root: &Path) -> Result<Self> {
        let path = Self::path_in(root);
        if !path.exists() {
            return Err(StackError::NotInitialized.into());
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read state file at {}", path.display()))?;
        let state = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse state file at {}", path.display()))?;
        Ok(state)
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = root.join(STACK_DIR);
        fs::create_dir_all(&dir)?;
        let path = Self::path_in(root);
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(&path, raw)?;
        Ok(())
    }

    pub fn branch(&self, name: &str) -> Option<&ManagedBranch> {
        self.branches.iter().find(|branch| branch.name == name)
    }

    pub fn branch_mut(&mut self, name: &str) -> Option<&mut ManagedBranch> {
        self.branches.iter_mut().find(|branch| branch.name == name)
    }

    pub fn add_branch(&mut self, branch: ManagedBranch) -> Result<()> {
        if self.branch(&branch.name).is_some() {
            return Err(StackError::BranchAlreadyTracked(branch.name).into());
        }
        self.branches.push(branch);
        Ok(())
    }

    pub fn remove_branch(&mut self, name: &str) {
        self.branches.retain(|branch| branch.name != name);
    }

    pub fn validate(&self, repo: &GitRepo) -> Result<()> {
        if self.version != 1 {
            anyhow::bail!("unsupported state version: {}", self.version);
        }
        if self
            .branches
            .iter()
            .any(|branch| branch.name == self.repo.trunk)
        {
            return Err(StackError::InvalidGraph("trunk branch cannot be tracked".into()).into());
        }
        graph::build(self)?;
        repo.ensure_branch_exists(&self.repo.trunk)?;
        for branch in &self.branches {
            if branch.parent != self.repo.trunk && self.branch(&branch.parent).is_none() {
                return Err(StackError::InvalidGraph(format!(
                    "parent '{}' for branch '{}' is not tracked and is not trunk",
                    branch.parent, branch.name
                ))
                .into());
            }
            if branch.status != BranchStatus::Archived {
                repo.ensure_branch_exists(&branch.name)?;
            }
            if branch.parent != self.repo.trunk {
                repo.ensure_branch_exists(&branch.parent)?;
            }
            repo.ensure_commit_exists(&branch.recorded_parent_tip)?;
        }
        Ok(())
    }
}

impl ManagedBranch {
    pub fn new(name: String, parent: String, recorded_parent_tip: String) -> Self {
        Self {
            name,
            parent,
            recorded_parent_tip,
            status: BranchStatus::Active,
            pr: None,
        }
    }
}

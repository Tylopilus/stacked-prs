use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::StackError;
use crate::git::GitRepo;
use crate::graph;

const STACK_DIR: &str = ".stacked-prs";
const STATE_FILE: &str = "state.json";
const PENDING_FILE: &str = "pending.json";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingReparent {
    pub operation: String,
    pub branch: String,
    pub old_parent: String,
    pub new_parent: String,
    pub old_parent_tip: String,
    pub new_parent_tip: String,
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
        self.validate_metadata(repo)?;
        for branch in &self.branches {
            if branch.status != BranchStatus::Archived {
                repo.ensure_branch_exists(&branch.name)?;
            }
            if branch.parent != self.repo.trunk {
                repo.ensure_branch_exists(&branch.parent)?;
            }
        }
        Ok(())
    }

    pub fn validate_metadata(&self, repo: &GitRepo) -> Result<()> {
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
            repo.ensure_commit_exists(&branch.recorded_parent_tip)?;
        }
        Ok(())
    }
}

impl PendingReparent {
    pub fn new(
        branch: String,
        old_parent: String,
        new_parent: String,
        old_parent_tip: String,
        new_parent_tip: String,
    ) -> Self {
        Self {
            operation: "reparent".to_string(),
            branch,
            old_parent,
            new_parent,
            old_parent_tip,
            new_parent_tip,
        }
    }

    pub fn path_in(root: &Path) -> PathBuf {
        root.join(STACK_DIR).join(PENDING_FILE)
    }

    pub fn load(root: &Path) -> Result<Self> {
        let path = Self::path_in(root);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read pending operation at {}", path.display()))?;
        let pending = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse pending operation at {}", path.display()))?;
        Ok(pending)
    }

    pub fn load_optional(root: &Path) -> Result<Option<Self>> {
        let path = Self::path_in(root);
        if path.exists() {
            Ok(Some(Self::load(root)?))
        } else {
            Ok(None)
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let dir = root.join(STACK_DIR);
        fs::create_dir_all(&dir)?;
        let raw = serde_json::to_string_pretty(self)?;
        fs::write(Self::path_in(root), raw)?;
        Ok(())
    }

    pub fn clear(root: &Path) -> Result<()> {
        let path = Self::path_in(root);
        if path.exists() {
            fs::remove_file(path)?;
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

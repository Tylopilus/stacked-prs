use anyhow::Result;

use crate::git::GitRepo;
use crate::graph;
use crate::state::{BranchStatus, RepoState};

pub fn cleanup(repo: &GitRepo, dry_run: bool) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    state.validate(repo)?;
    let current = repo.current_branch()?;

    let mut candidates = Vec::new();
    for branch in &state.branches {
        if branch.status != BranchStatus::Merged {
            continue;
        }
        if branch.name == current {
            continue;
        }
        if !graph::active_children(&state, &branch.name).is_empty() {
            continue;
        }
        if repo.branch_exists(&branch.name)? {
            candidates.push(branch.name.clone());
        }
    }

    if candidates.is_empty() {
        println!("No merged branches eligible for cleanup");
        return Ok(());
    }

    for branch in &candidates {
        println!("cleanup candidate: {branch}");
    }

    if dry_run {
        return Ok(());
    }

    for branch in candidates {
        repo.delete_branch(&branch)?;
        state.remove_branch(&branch);
    }
    state.save(&repo.root)?;
    println!("Cleanup complete");
    Ok(())
}

use anyhow::Result;

use crate::git::GitRepo;
use crate::graph;
use crate::state::{BranchStatus, RepoState};

pub fn cleanup(repo: &GitRepo, dry_run: bool) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    state.validate_metadata(repo)?;
    let current = repo.current_branch()?;

    let mut delete_candidates = Vec::new();
    let mut prune_candidates = Vec::new();
    for branch in &state.branches {
        if branch.name == current {
            continue;
        }
        let branch_exists = repo.branch_exists(&branch.name)?;
        if !branch_exists {
            if !has_existing_descendant(repo, &state, &branch.name)? {
                prune_candidates.push(branch.name.clone());
            }
            continue;
        }
        if branch.status == BranchStatus::Merged
            && graph::active_children(&state, &branch.name).is_empty()
        {
            delete_candidates.push(branch.name.clone());
        }
    }

    if delete_candidates.is_empty() && prune_candidates.is_empty() {
        println!("No merged branches eligible for cleanup");
        return Ok(());
    }

    for branch in &delete_candidates {
        println!("cleanup candidate: {branch}");
    }
    for branch in &prune_candidates {
        println!("state prune candidate: {branch} (local branch missing)");
    }

    if dry_run {
        return Ok(());
    }

    for branch in delete_candidates {
        repo.delete_branch(&branch)?;
        state.remove_branch(&branch);
    }
    for branch in prune_candidates {
        state.remove_branch(&branch);
    }
    state.save(&repo.root)?;
    println!("Cleanup complete");
    Ok(())
}

fn has_existing_descendant(repo: &GitRepo, state: &RepoState, branch_name: &str) -> Result<bool> {
    for branch in &state.branches {
        if branch.name == branch_name || !repo.branch_exists(&branch.name)? {
            continue;
        }
        let mut parent = branch.parent.as_str();
        while parent != state.repo.trunk {
            if parent == branch_name {
                return Ok(true);
            }
            let Some(parent_branch) = state.branch(parent) else {
                break;
            };
            parent = &parent_branch.parent;
        }
    }
    Ok(false)
}

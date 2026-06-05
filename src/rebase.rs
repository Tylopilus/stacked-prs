use anyhow::Result;

use crate::state::{BranchStatus, RepoState};
use crate::{git::GitRepo, output};

#[derive(Debug, Clone)]
pub struct RebasePlan {
    pub branch: String,
    pub configured_parent: String,
    pub effective_parent: String,
    pub old_base: String,
    pub new_base: String,
}

pub fn effective_parent(state: &RepoState, branch: &crate::state::ManagedBranch) -> String {
    if branch.parent == state.repo.trunk {
        return state.repo.trunk.clone();
    }
    match state.branch(&branch.parent).map(|parent| &parent.status) {
        Some(BranchStatus::Merged) => state.repo.trunk.clone(),
        _ => branch.parent.clone(),
    }
}

pub fn rebase_target(repo: &GitRepo, effective_parent: &str, trunk: &str) -> Result<String> {
    let remote_tracking = format!("origin/{}", effective_parent);
    if repo.branch_exists(&remote_tracking)? {
        return Ok(remote_tracking);
    }
    if effective_parent == trunk {
        if let Ok(local_tip) = repo.branch_tip(effective_parent) {
            return Ok(local_tip);
        }
    }
    repo.branch_tip(effective_parent)
}

pub fn plan_rebase(
    repo: &GitRepo,
    state: &RepoState,
    branch_name: &str,
    onto: Option<&str>,
) -> Result<Option<RebasePlan>> {
    let branch = state
        .branch(branch_name)
        .ok_or_else(|| anyhow::anyhow!("branch not tracked: {branch_name}"))?;
    let effective_parent = onto
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| effective_parent(state, branch));
    let new_base = rebase_target(repo, &effective_parent, &state.repo.trunk)?;
    let old_base = branch.recorded_parent_tip.clone();
    if old_base == new_base {
        return Ok(None);
    }

    Ok(Some(RebasePlan {
        branch: branch.name.clone(),
        configured_parent: branch.parent.clone(),
        effective_parent,
        old_base,
        new_base,
    }))
}

pub fn rebase_branch(
    repo: &GitRepo,
    branch_name: &str,
    onto: Option<&str>,
    dry_run: bool,
) -> Result<()> {
    repo.ensure_clean()?;
    let mut state = RepoState::load(&repo.root)?;
    state.validate(repo)?;

    let Some(plan) = plan_rebase(repo, &state, branch_name, onto)? else {
        println!("Branch is already up to date");
        return Ok(());
    };

    output::print_rebase_plan(&plan);
    if dry_run {
        return Ok(());
    }

    repo.rebase_onto(&plan.new_base, &plan.old_base, &plan.branch)?;

    let branch = state
        .branch_mut(&plan.branch)
        .ok_or_else(|| anyhow::anyhow!("branch disappeared from state: {}", plan.branch))?;
    branch.recorded_parent_tip = plan.new_base;
    state.save(&repo.root)?;
    println!("Rebased {}", plan.branch);
    Ok(())
}

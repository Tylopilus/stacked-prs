use anyhow::Result;

use crate::git::GitRepo;
use crate::graph;
use crate::output;
use crate::rebase::{RebasePlan, plan_rebase};
use crate::state::{BranchStatus, RepoState};

pub fn sync_all(repo: &GitRepo, all: bool, dry_run: bool) -> Result<()> {
    if !all {
        anyhow::bail!("sync currently requires --all");
    }
    repo.ensure_clean()?;
    let mut state = RepoState::load(&repo.root)?;
    state.validate(repo)?;

    if repo.fetch(&state.repo.remote).is_err() {
        println!("Fetch failed; continuing with local refs only");
    }

    let mut plans = Vec::<RebasePlan>::new();
    for branch_name in graph::descendants_topo(&state)? {
        let branch = state
            .branch(branch_name)
            .ok_or_else(|| anyhow::anyhow!("branch disappeared from state: {branch_name}"))?;
        if branch.status != BranchStatus::Active {
            continue;
        }
        if let Some(plan) = plan_rebase(repo, &state, branch_name, None)? {
            plans.push(plan);
        }
    }

    if plans.is_empty() {
        println!("All tracked branches are up to date");
        return Ok(());
    }

    for plan in &plans {
        output::print_rebase_plan(plan);
    }

    if dry_run {
        return Ok(());
    }

    for plan in plans {
        repo.rebase_onto(&plan.new_base, &plan.old_base, &plan.branch)?;
        let branch = state
            .branch_mut(&plan.branch)
            .ok_or_else(|| anyhow::anyhow!("branch disappeared from state: {}", plan.branch))?;
        branch.recorded_parent_tip = plan.new_base;
        state.save(&repo.root)?;
    }

    println!("Sync complete");
    Ok(())
}

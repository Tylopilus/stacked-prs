use anyhow::Result;

use crate::azure::{self, AzCli};
use crate::git::GitRepo;
use crate::graph;
use crate::output;
use crate::rebase::{RebasePlan, plan_rebase};
use crate::state::{BranchStatus, RepoState};

pub fn sync_all(repo: &GitRepo, all: bool, dry_run: bool, push: bool, no_pr: bool) -> Result<()> {
    if !all {
        anyhow::bail!("sync currently requires --all");
    }
    repo.ensure_clean()?;
    let mut state = RepoState::load(&repo.root)?;
    state.validate(repo)?;

    let starting_branch = repo.current_branch()?;

    if repo.fetch(&state.repo.remote).is_err() {
        println!("Fetch failed; continuing with local refs only");
    }

    // Reconcile with Azure DevOps before planning: completed PRs mark their
    // branches as merged automatically, so effective parents redirect to
    // trunk without a manual `stack mark-merged`.
    let has_prs = state.branches.iter().any(|branch| branch.pr.is_some());
    if !no_pr && has_prs {
        let az = AzCli::new(repo);
        let mut reconciled_state = state.clone();
        match azure::reconcile(&az, &mut reconciled_state, !dry_run) {
            Ok(changed) => {
                if changed && !dry_run {
                    reconciled_state.save(&repo.root)?;
                }
                state = reconciled_state;
            }
            Err(err) => {
                println!("Warning: PR reconciliation failed ({err:#}); continuing with local state")
            }
        }
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
        if dry_run || !push {
            return Ok(());
        }
    } else {
        for plan in &plans {
            output::print_rebase_plan(plan);
        }

        if dry_run {
            return Ok(());
        }

        for plan in &plans {
            repo.rebase_onto(&plan.new_base, &plan.old_base, &plan.branch)?;
            let branch = state
                .branch_mut(&plan.branch)
                .ok_or_else(|| anyhow::anyhow!("branch disappeared from state: {}", plan.branch))?;
            branch.recorded_parent_tip = plan.new_base.clone();
            state.save(&repo.root)?;
        }
    }

    if starting_branch != state.repo.trunk {
        repo.checkout(&starting_branch)?;
    }

    if push {
        for branch in &state.branches {
            if branch.status != BranchStatus::Active {
                continue;
            }
            repo.push(&state.repo.remote, &branch.name, true)?;
            println!("Pushed {} (force-with-lease)", branch.name);
        }
    }

    // Refresh the stack overview block in PR descriptions after restacking.
    if !no_pr && has_prs {
        let az = AzCli::new(repo);
        if let Err(err) = azure::update_stack_descriptions(&az, &state) {
            println!("Warning: failed to update PR descriptions: {err:#}");
        }
    }

    let trunk = &state.repo.trunk;
    let remote_trunk = format!("origin/{}", trunk);
    if repo.branch_exists(&remote_trunk)? {
        let local_tip = repo.branch_tip(trunk)?;
        let remote_tip = repo.branch_tip(&remote_trunk)?;
        if local_tip != remote_tip {
            println!(
                "Tip: local {trunk} is behind {remote_trunk}. Run 'git checkout {trunk} && git pull' to update."
            );
        }
    }

    println!("Sync complete");
    Ok(())
}

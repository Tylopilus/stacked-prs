use anyhow::Result;

use crate::azure::{self, AzCli};
use crate::git::GitRepo;
use crate::graph;
use crate::output;
use crate::rebase::{RebasePlan, plan_rebase};
use crate::state::{BranchStatus, RepoState};

pub fn sync_all(
    repo: &GitRepo,
    _all: bool,
    continue_sync: bool,
    dry_run: bool,
    push: bool,
    no_pr: bool,
) -> Result<()> {
    if continue_sync {
        return sync_continue(repo, push, no_pr);
    }
    repo.ensure_clean()?;
    let mut state = RepoState::load(&repo.root)?;
    state.validate(repo)?;

    let starting_branch = repo.current_branch()?;

    if repo.fetch(&state.repo.remote).is_err() {
        println!("Fetch failed; continuing with local refs only");
    }

    // Reconcile PR statuses before planning: completed PRs mark their branches
    // as merged automatically, so effective parents redirect to trunk without a
    // manual `stack mark-merged`. Do not retarget PRs yet; children must be
    // rebased and pushed first or Azure shows the merged lower-stack diff again.
    let has_prs = state.branches.iter().any(|branch| branch.pr.is_some());
    if !no_pr && has_prs {
        let az = AzCli::new(repo);
        let mut reconciled_state = state.clone();
        match azure::reconcile_statuses(&az, &mut reconciled_state) {
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

    let branch_order: Vec<String> = graph::descendants_topo(&state)?
        .iter()
        .map(|branch| (*branch).to_string())
        .collect();

    if dry_run {
        let mut plans = Vec::<RebasePlan>::new();
        for branch_name in &branch_order {
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
        } else {
            for plan in &plans {
                output::print_rebase_plan(plan);
            }
        }
        return Ok(());
    }

    let mut rebased = false;
    for branch_name in &branch_order {
        let branch = state
            .branch(branch_name)
            .ok_or_else(|| anyhow::anyhow!("branch disappeared from state: {branch_name}"))?;
        if branch.status != BranchStatus::Active {
            continue;
        }
        if let Some(plan) = plan_rebase(repo, &state, branch_name, None)? {
            output::print_rebase_plan(&plan);
            repo.rebase_onto(&plan.new_base, &plan.old_base, &plan.branch)?;
            let branch = state
                .branch_mut(&plan.branch)
                .ok_or_else(|| anyhow::anyhow!("branch disappeared from state: {}", plan.branch))?;
            branch.recorded_parent_tip = plan.new_base.clone();
            state.save(&repo.root)?;
            rebased = true;
        }
    }

    if !rebased {
        println!("All tracked branches are up to date");
        if !push {
            return Ok(());
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

    // Now that rebased branches have been pushed, it is safe to retarget PRs.
    if !no_pr && has_prs && !dry_run {
        let az = AzCli::new(repo);
        if let Err(err) = azure::reconcile(&az, &mut state, true) {
            println!("Warning: PR retargeting failed ({err:#}); continuing")
        } else {
            state.save(&repo.root)?;
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

fn sync_continue(repo: &GitRepo, push: bool, no_pr: bool) -> Result<()> {
    let Some((branch_name, new_base)) = repo.rebase_branch_and_onto()? else {
        anyhow::bail!("no stack sync rebase in progress");
    };

    let unmerged_paths = repo.unmerged_paths()?;
    if !unmerged_paths.is_empty() {
        let unresolved = repo.paths_with_conflict_markers(&unmerged_paths)?;
        if !unresolved.is_empty() {
            anyhow::bail!(
                "conflicts are not fully resolved in: {}",
                unresolved.join(", ")
            );
        }
        repo.add_paths(&unmerged_paths)?;
        println!(
            "Staged resolved conflict file(s): {}",
            unmerged_paths.join(", ")
        );
    }

    if let Err(err) = repo.rebase_continue() {
        println!();
        println!("Rebase is still not complete.");
        println!("Resolve conflicts, then run: git stack sync --continue --push");
        return Err(err);
    }

    if repo.is_rebase_in_progress()? {
        println!("Rebase continued; more commits remain.");
        println!("If conflicts appear, resolve them and run: git stack sync --continue --push");
        return Ok(());
    }

    let mut state = RepoState::load(&repo.root)?;
    let branch = state
        .branch_mut(&branch_name)
        .ok_or_else(|| anyhow::anyhow!("rebased branch is not tracked: {branch_name}"))?;
    branch.recorded_parent_tip = new_base;
    state.save(&repo.root)?;
    println!("Recorded completed rebase for {branch_name}");

    sync_all(repo, true, false, false, push, no_pr)
}

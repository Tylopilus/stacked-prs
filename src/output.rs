use anyhow::Result;
use serde::Serialize;

use crate::rebase::{RebasePlan, effective_parent};
use crate::state::{BranchStatus, RepoState};
use crate::{git::GitRepo, graph};

#[derive(Debug, Clone, Serialize)]
pub struct BranchStatusReport {
    pub name: String,
    pub parent: String,
    pub effective_parent: String,
    pub status: String,
    pub drift: String,
    pub recorded_parent_tip: String,
    pub current_parent_tip: String,
    pub pr_target: String,
}

pub fn build_status_report(repo: &GitRepo, state: &RepoState) -> Result<Vec<BranchStatusReport>> {
    let mut reports = Vec::new();
    for branch_name in graph::descendants_topo(state)? {
        let branch = state
            .branch(branch_name)
            .expect("branch should exist in report build");
        let effective_parent = effective_parent(state, branch);
        let branch_exists = repo.branch_exists(&branch.name)?;
        let current_parent_tip = repo.branch_tip(&effective_parent).ok();
        let drift = if !branch_exists {
            "missing".to_string()
        } else if current_parent_tip.is_none() {
            "parent_missing".to_string()
        } else if branch.status == BranchStatus::Merged {
            "merged".to_string()
        } else if branch.recorded_parent_tip == *current_parent_tip.as_ref().unwrap() {
            "up_to_date".to_string()
        } else {
            "needs_rebase".to_string()
        };
        let status = if branch_exists {
            format_status(&branch.status)
        } else {
            "missing".to_string()
        };
        reports.push(BranchStatusReport {
            name: branch.name.clone(),
            parent: branch.parent.clone(),
            effective_parent: effective_parent.clone(),
            status,
            drift,
            recorded_parent_tip: short_sha(&branch.recorded_parent_tip),
            current_parent_tip: current_parent_tip
                .as_deref()
                .map(short_sha)
                .unwrap_or_else(|| "unknown".to_string()),
            pr_target: effective_parent,
        });
    }
    Ok(reports)
}

pub fn print_status_text(state: &RepoState, report: &[BranchStatusReport]) {
    println!("{}", state.repo.trunk);
    for branch in report {
        println!("- {} [{}]", branch.name, branch.status);
        println!("  parent: {}", branch.parent);
        println!("  effective parent: {}", branch.effective_parent);
        println!("  drift: {}", branch.drift);
        println!("  recorded parent tip: {}", branch.recorded_parent_tip);
        println!("  current parent tip: {}", branch.current_parent_tip);
        println!("  pr target: {}", branch.pr_target);
    }
}

pub fn print_status_json(report: &[BranchStatusReport]) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(report)?);
    Ok(())
}

pub fn print_rebase_plan(plan: &RebasePlan) {
    println!(
        "rebase {}: parent={} effective_parent={} old_base={} new_base={}",
        plan.branch,
        plan.configured_parent,
        plan.effective_parent,
        short_sha(&plan.old_base),
        short_sha(&plan.new_base)
    );
}

pub fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn format_status(status: &BranchStatus) -> String {
    match status {
        BranchStatus::Active => "active".to_string(),
        BranchStatus::Merged => "merged".to_string(),
        BranchStatus::Archived => "archived".to_string(),
    }
}

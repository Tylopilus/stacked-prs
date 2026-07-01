use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::error::StackError;
use crate::state::RepoState;

pub struct StackGraph<'a> {
    pub children: BTreeMap<&'a str, Vec<&'a str>>,
}

pub fn build(state: &RepoState) -> Result<StackGraph<'_>> {
    let mut names = BTreeSet::new();
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for branch in &state.branches {
        if !names.insert(branch.name.as_str()) {
            return Err(
                StackError::InvalidGraph(format!("duplicate branch '{}'", branch.name)).into(),
            );
        }
        if branch.name == branch.parent {
            return Err(StackError::InvalidGraph(format!(
                "branch '{}' cannot parent itself",
                branch.name
            ))
            .into());
        }
        children
            .entry(branch.parent.as_str())
            .or_default()
            .push(branch.name.as_str());
        children.entry(branch.name.as_str()).or_default();
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for branch in &state.branches {
        dfs(branch.name.as_str(), state, &mut visiting, &mut visited)?;
    }

    for child_list in children.values_mut() {
        child_list.sort_unstable();
    }

    Ok(StackGraph { children })
}

fn dfs<'a>(
    branch: &'a str,
    state: &'a RepoState,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> Result<()> {
    if visited.contains(branch) {
        return Ok(());
    }
    if !visiting.insert(branch) {
        return Err(StackError::InvalidGraph(format!("cycle detected at '{branch}'")).into());
    }
    let managed = state
        .branch(branch)
        .ok_or_else(|| StackError::InvalidGraph(format!("unknown branch '{branch}'")))?;
    if managed.parent != state.repo.trunk && state.branch(&managed.parent).is_some() {
        dfs(managed.parent.as_str(), state, visiting, visited)?;
    }
    visiting.remove(branch);
    visited.insert(branch);
    Ok(())
}

pub fn descendants_topo<'a>(state: &'a RepoState) -> Result<Vec<&'a str>> {
    let graph = build(state)?;
    let mut ordered = Vec::new();
    let mut roots: Vec<&str> = state
        .branches
        .iter()
        .filter(|branch| {
            branch.parent == state.repo.trunk || state.branch(&branch.parent).is_none()
        })
        .map(|branch| branch.name.as_str())
        .collect();
    roots.sort_unstable();
    for root in roots {
        visit(root, &graph, &mut ordered);
    }
    Ok(ordered)
}

pub fn descendants_of<'a>(state: &'a RepoState, parent: &str) -> Result<Vec<&'a str>> {
    if parent != state.repo.trunk && state.branch(parent).is_none() {
        return Err(crate::error::StackError::InvalidGraph(format!(
            "branch is not tracked: {parent}"
        ))
        .into());
    }

    let graph = build(state)?;
    let mut ordered = Vec::new();
    if let Some(children) = graph.children.get(parent) {
        for child in children {
            visit(child, &graph, &mut ordered);
        }
    }
    Ok(ordered)
}

fn visit<'a>(branch: &'a str, graph: &StackGraph<'a>, ordered: &mut Vec<&'a str>) {
    ordered.push(branch);
    if let Some(children) = graph.children.get(branch) {
        for child in children {
            visit(child, graph, ordered);
        }
    }
}

pub fn active_children<'a>(state: &'a RepoState, parent: &str) -> Vec<&'a str> {
    state
        .branches
        .iter()
        .filter(|branch| {
            branch.parent == parent && branch.status == crate::state::BranchStatus::Active
        })
        .map(|branch| branch.name.as_str())
        .collect()
}

use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::git::GitRepo;
use crate::graph;
use crate::rebase::effective_parent;
use crate::state::{BranchStatus, PullRequestRef, RepoState};

const STACK_BEGIN: &str = "<!-- stack:begin -->";
const STACK_END: &str = "<!-- stack:end -->";
const PROVIDER: &str = "azure-devops";
const AZURE_DEVOPS_RESOURCE: &str = "499b84ac-1321-427f-aa17-267ca6975798";

/// Subset of the JSON returned by `az repos pr show/create -o json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub pull_request_id: u64,
    pub url: Option<String>,
    /// "active" | "completed" | "abandoned"
    pub status: String,
    pub target_ref_name: String,
    pub description: Option<String>,
    pub is_draft: Option<bool>,
    #[serde(default)]
    pub reviewers: Vec<Reviewer>,
    pub repository: Option<RepoInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reviewer {
    pub vote: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub url: Option<String>,
    pub web_url: Option<String>,
}

impl PrInfo {
    pub fn web_url(&self) -> Option<String> {
        self.repository
            .as_ref()
            .and_then(|repo| repo.web_url.as_ref())
            .map(|base| format!("{}/pullrequest/{}", base, self.pull_request_id))
    }

    pub fn target_branch(&self) -> &str {
        strip_ref(&self.target_ref_name)
    }

    fn api_url(&self) -> Result<String> {
        if let Some(url) = &self.url {
            return Ok(url.clone());
        }
        let repo_url = self
            .repository
            .as_ref()
            .and_then(|repo| repo.url.as_ref())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot retarget PR !{}: az output did not include REST URL metadata",
                    self.pull_request_id
                )
            })?;
        Ok(format!("{repo_url}/pullRequests/{}", self.pull_request_id))
    }
}

fn strip_ref(name: &str) -> &str {
    name.strip_prefix("refs/heads/").unwrap_or(name)
}

fn branch_ref(name: &str) -> String {
    if name.starts_with("refs/heads/") {
        name.to_string()
    } else {
        format!("refs/heads/{name}")
    }
}

/// Thin wrapper around the `az` CLI, run from the repository root so that
/// organization/project/repository are auto-detected from the git remote.
pub struct AzCli<'a> {
    repo: &'a GitRepo,
}

impl<'a> AzCli<'a> {
    pub fn new(repo: &'a GitRepo) -> Self {
        Self { repo }
    }

    pub fn pr_show(&self, id: &str) -> Result<PrInfo> {
        let raw = self.az(&["repos", "pr", "show", "--id", id, "-o", "json"])?;
        let info: PrInfo = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse az output for PR {id}"))?;
        Ok(info)
    }

    pub fn pr_create(
        &self,
        source: &str,
        target: &str,
        title: &str,
        draft: bool,
    ) -> Result<PrInfo> {
        let mut args = vec![
            "repos",
            "pr",
            "create",
            "--source-branch",
            source,
            "--target-branch",
            target,
            "--title",
            title,
        ];
        if draft {
            args.extend(["--draft", "true"]);
        }
        args.extend(["-o", "json"]);
        let raw = self.az(&args)?;
        let info: PrInfo = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse az output creating PR for {source}"))?;
        Ok(info)
    }

    pub fn pr_update_description(&self, id: &str, description: &str) -> Result<()> {
        let description_arg = format!("--description={description}");
        let args = [
            "repos",
            "pr",
            "update",
            "--id",
            id,
            description_arg.as_str(),
            "-o",
            "none",
        ];
        self.az(&args)?;
        Ok(())
    }

    pub fn pr_retarget(&self, info: &PrInfo, target: &str) -> Result<()> {
        let body = serde_json::json!({ "targetRefName": branch_ref(target) }).to_string();
        let url = format!("{}?api-version=7.1", info.api_url()?);
        self.az(&[
            "rest",
            "--resource",
            AZURE_DEVOPS_RESOURCE,
            "--method",
            "PATCH",
            "--url",
            url.as_str(),
            "--body",
            body.as_str(),
            "--headers",
            "Content-Type=application/json",
            "-o",
            "none",
        ])?;
        Ok(())
    }

    pub fn pr_set_autocomplete(&self, id: &str) -> Result<()> {
        self.az(&[
            "repos",
            "pr",
            "update",
            "--id",
            id,
            "--auto-complete",
            "true",
            "--squash",
            "true",
            "--delete-source-branch",
            "true",
            "-o",
            "none",
        ])?;
        Ok(())
    }

    fn az(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("az")
            .args(args)
            .current_dir(&self.repo.root)
            .output()
            .with_context(|| {
                format!(
                    "failed to run az {} (is the Azure CLI installed and are you logged in?)",
                    args.join(" ")
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("az {} failed: {}", args.join(" "), stderr.trim());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }
}

/// `stack pr create [branch]`: push the branch and open a PR against its
/// effective parent, recording the PR in stack state.
pub fn pr_create(
    repo: &GitRepo,
    branch: Option<String>,
    title: Option<String>,
    draft: bool,
) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    state.validate(repo)?;

    let branch = match branch {
        Some(branch) => branch,
        None => repo.current_branch()?,
    };
    let managed = state
        .branch(&branch)
        .ok_or_else(|| anyhow::anyhow!("branch not tracked: {branch}"))?;
    if managed.status != BranchStatus::Active {
        anyhow::bail!("branch is not active: {branch}");
    }
    if let Some(pr) = &managed.pr {
        anyhow::bail!("branch already has a tracked PR: !{}", pr.id);
    }

    let target = effective_parent(&state, managed);
    repo.push(&state.repo.remote, &branch, false)?;

    let title = title.unwrap_or_else(|| branch.clone());
    let az = AzCli::new(repo);
    let info = az.pr_create(&branch, &target, &title, draft)?;
    let url = info.web_url();

    let managed = state
        .branch_mut(&branch)
        .expect("branch existence checked above");
    managed.pr = Some(PullRequestRef {
        provider: PROVIDER.to_string(),
        id: info.pull_request_id.to_string(),
        url: url.clone(),
        target_branch: Some(target.clone()),
    });
    state.save(&repo.root)?;

    println!(
        "Created PR !{} for {} -> {}",
        info.pull_request_id, branch, target
    );
    if let Some(url) = url {
        println!("  {url}");
    }

    update_stack_descriptions(&az, &state)?;
    Ok(())
}

/// `stack pr sync`: reconcile PR completion/abandonment with local state and
/// refresh the stack overview block in every open PR description.
pub fn pr_sync(repo: &GitRepo) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    state.validate_metadata(repo)?;
    let az = AzCli::new(repo);
    let changed = reconcile_statuses(&az, &mut state)?;
    if changed {
        state.save(&repo.root)?;
    }
    update_stack_descriptions(&az, &state)?;
    if !changed {
        println!("PR state is in sync");
    }
    Ok(())
}

/// Pull PR status from Azure DevOps and fold it back into stack state:
/// - completed PR -> branch marked merged (automates `stack mark-merged`)
/// - abandoned PR -> PR reference dropped from state
/// - active PR whose target drifted from the effective parent -> retargeted
///   (only when `apply_remote` is true; dry runs just report)
///
/// Returns whether state changed. The caller decides when to save.
pub fn reconcile(az: &AzCli, state: &mut RepoState, apply_remote: bool) -> Result<bool> {
    reconcile_inner(az, state, apply_remote, true)
}

/// Reconcile only PR statuses. This is used before rebase planning so completed
/// lower-stack PRs redirect effective parents without retargeting children yet.
pub fn reconcile_statuses(az: &AzCli, state: &mut RepoState) -> Result<bool> {
    reconcile_inner(az, state, false, false)
}

fn reconcile_inner(
    az: &AzCli,
    state: &mut RepoState,
    apply_remote: bool,
    retarget_prs: bool,
) -> Result<bool> {
    let mut changed = false;
    let mut infos: BTreeMap<String, PrInfo> = BTreeMap::new();

    let tracked: Vec<(String, String)> = state
        .branches
        .iter()
        .filter_map(|branch| {
            branch
                .pr
                .as_ref()
                .map(|pr| (branch.name.clone(), pr.id.clone()))
        })
        .collect();

    for (name, pr_id) in &tracked {
        let info = az.pr_show(pr_id)?;
        match info.status.as_str() {
            "completed" => {
                let branch = state
                    .branch_mut(name)
                    .expect("tracked branch present in state");
                if branch.status != BranchStatus::Merged {
                    branch.status = BranchStatus::Merged;
                    println!("Marked {name} as merged (PR !{pr_id} completed)");
                    changed = true;
                }
            }
            "abandoned" => {
                let branch = state
                    .branch_mut(name)
                    .expect("tracked branch present in state");
                println!("PR !{pr_id} for {name} was abandoned; untracking the PR");
                branch.pr = None;
                changed = true;
            }
            _ => {
                infos.insert(name.clone(), info);
            }
        }
    }

    if retarget_prs {
        // Retarget pass, after merged statuses settled so effective parents are final.
        for (name, info) in &infos {
            let branch = state.branch(name).expect("tracked branch present in state");
            if branch.status != BranchStatus::Active {
                continue;
            }
            let expected = effective_parent(state, branch);
            let actual = info.target_branch().to_string();
            let pr_id = info.pull_request_id.to_string();
            if actual != expected {
                if apply_remote {
                    az.pr_retarget(info, &expected)?;
                    println!("Retargeted PR !{pr_id} ({name}): {actual} -> {expected}");
                } else {
                    println!("Would retarget PR !{pr_id} ({name}): {actual} -> {expected}");
                }
            }
            let branch = state
                .branch_mut(name)
                .expect("tracked branch present in state");
            let pr = branch.pr.as_mut().expect("PR ref present");
            if pr.target_branch.as_deref() != Some(expected.as_str()) {
                pr.target_branch = Some(expected);
                changed = true;
            }
        }
    }

    Ok(changed)
}

/// Rewrite the stack overview block (between sentinel markers) in every open
/// PR description. Editing descriptions never resets reviewer votes.
pub fn update_stack_descriptions(az: &AzCli, state: &RepoState) -> Result<()> {
    let order: Vec<String> = graph::descendants_topo(state)?
        .iter()
        .map(|name| (*name).to_string())
        .collect();

    let mut infos: BTreeMap<String, PrInfo> = BTreeMap::new();
    for name in &order {
        let Some(branch) = state.branch(name) else {
            continue;
        };
        if branch.status != BranchStatus::Active {
            continue;
        }
        let Some(pr) = &branch.pr else {
            continue;
        };
        match az.pr_show(&pr.id) {
            Ok(info) if info.status == "active" => {
                infos.insert(name.clone(), info);
            }
            Ok(_) => {}
            Err(err) => println!("Warning: could not fetch PR !{} for {name}: {err:#}", pr.id),
        }
    }

    for (name, info) in &infos {
        let stack_order = stack_order_for_branch(state, &order, name);
        let block = build_stack_block(state, &stack_order, &infos, name);
        let description = splice_block(info.description.as_deref().unwrap_or(""), &block);
        az.pr_update_description(&info.pull_request_id.to_string(), &description)?;
    }
    if !infos.is_empty() {
        println!("Updated stack description on {} PR(s)", infos.len());
    }
    Ok(())
}

fn stack_order_for_branch(state: &RepoState, order: &[String], current: &str) -> Vec<String> {
    let root = stack_root(state, current);
    order
        .iter()
        .filter(|name| branch_is_in_stack(state, name, &root))
        .cloned()
        .collect()
}

fn stack_root(state: &RepoState, current: &str) -> String {
    let mut root = current.to_string();
    loop {
        let Some(branch) = state.branch(&root) else {
            break;
        };
        if state.branch(&branch.parent).is_none() {
            break;
        }
        root = branch.parent.clone();
    }
    root
}

fn branch_is_in_stack(state: &RepoState, name: &str, root: &str) -> bool {
    let mut candidate = name;
    loop {
        if candidate == root {
            return true;
        }
        let Some(branch) = state.branch(candidate) else {
            return false;
        };
        if state.branch(&branch.parent).is_none() {
            return false;
        }
        candidate = &branch.parent;
    }
}

fn build_stack_block(
    state: &RepoState,
    order: &[String],
    infos: &BTreeMap<String, PrInfo>,
    current: &str,
) -> String {
    let mut lines = vec![
        STACK_BEGIN.to_string(),
        "📚 **Stack** (merge order, bottom first):".to_string(),
        String::new(),
        format!("`{}`", state.repo.trunk),
    ];
    for name in order {
        let Some(branch) = state.branch(name) else {
            continue;
        };
        let pr_label = branch
            .pr
            .as_ref()
            .map(|pr| format!("!{}", pr.id))
            .unwrap_or_else(|| "(no PR)".to_string());
        let status = match branch.status {
            BranchStatus::Merged => "✅ merged".to_string(),
            BranchStatus::Archived => "📦 archived".to_string(),
            BranchStatus::Active => infos
                .get(name)
                .map(vote_summary)
                .unwrap_or_else(|| "⏳".to_string()),
        };
        let here = if name == current {
            "  👈 this PR"
        } else {
            ""
        };
        lines.push(format!("← {pr_label} `{name}` — {status}{here}"));
    }
    lines.push(String::new());
    lines.push("Review and merge bottom-up. This block is auto-generated by `stack`.".to_string());
    lines.push(STACK_END.to_string());
    lines.join("\n")
}

fn vote_summary(info: &PrInfo) -> String {
    if info.is_draft.unwrap_or(false) {
        return "📝 draft".to_string();
    }
    let votes: Vec<i32> = info
        .reviewers
        .iter()
        .map(|reviewer| reviewer.vote)
        .collect();
    if votes.iter().any(|vote| *vote == -10) {
        return "❌ rejected".to_string();
    }
    if votes.iter().any(|vote| *vote == -5) {
        return "✏️ changes requested".to_string();
    }
    let approvals = votes.iter().filter(|vote| **vote >= 5).count();
    if approvals > 0 {
        format!("✅ approved ({approvals})")
    } else {
        "⏳ awaiting review".to_string()
    }
}

fn splice_block(description: &str, block: &str) -> String {
    if let (Some(start), Some(end)) = (description.find(STACK_BEGIN), description.find(STACK_END)) {
        if end >= start {
            let end_total = end + STACK_END.len();
            return format!(
                "{}{}{}",
                &description[..start],
                block,
                &description[end_total..]
            );
        }
    }
    if description.trim().is_empty() {
        block.to_string()
    } else {
        format!("{description}\n\n{block}")
    }
}

/// `stack land [branch]`: set auto-complete (squash + delete source branch)
/// on the bottom PR of the stack. Azure DevOps merges it once policies pass
/// and auto-retargets its children when the source branch is deleted.
pub fn land(repo: &GitRepo, branch: Option<String>) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    state.validate_metadata(repo)?;
    let az = AzCli::new(repo);

    // Make sure we are not landing something that already merged.
    let changed = reconcile(&az, &mut state, true)?;
    if changed {
        state.save(&repo.root)?;
    }

    let branch = match branch {
        Some(branch) => branch,
        None => {
            let roots: Vec<String> = state
                .branches
                .iter()
                .filter(|branch| {
                    branch.status == BranchStatus::Active
                        && branch.pr.is_some()
                        && effective_parent(&state, branch) == state.repo.trunk
                })
                .map(|branch| branch.name.clone())
                .collect();
            match roots.len() {
                0 => anyhow::bail!("no active branch with a PR targets {}", state.repo.trunk),
                1 => roots.into_iter().next().expect("one root"),
                _ => anyhow::bail!(
                    "multiple branches with PRs target {}: {}; pass one explicitly",
                    state.repo.trunk,
                    roots.join(", ")
                ),
            }
        }
    };

    let managed = state
        .branch(&branch)
        .ok_or_else(|| anyhow::anyhow!("branch not tracked: {branch}"))?;
    if managed.status != BranchStatus::Active {
        anyhow::bail!("branch is not active: {branch}");
    }
    let target = effective_parent(&state, managed);
    if target != state.repo.trunk {
        anyhow::bail!(
            "{branch} targets {target}, not trunk; land the stack bottom-up starting at the trunk-facing PR"
        );
    }
    let pr_id = managed
        .pr
        .as_ref()
        .map(|pr| pr.id.clone())
        .ok_or_else(|| anyhow::anyhow!("branch has no tracked PR: {branch}"))?;

    az.pr_set_autocomplete(&pr_id)?;
    println!(
        "Auto-complete set on PR !{pr_id} ({branch} -> {}): squash + delete source branch.",
        state.repo.trunk
    );
    println!("Once it completes, run 'stack sync --all --push' to restack the rest.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_az_pr_json() {
        let raw = r#"{
            "pullRequestId": 42,
            "status": "active",
            "targetRefName": "refs/heads/feat-a",
            "sourceRefName": "refs/heads/feat-b",
            "description": "hello",
            "isDraft": false,
            "reviewers": [{"vote": 10, "displayName": "x"}, {"vote": 0}],
            "repository": {"webUrl": "https://dev.azure.com/org/proj/_git/repo"},
            "mergeStatus": "succeeded"
        }"#;
        let info: PrInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(info.pull_request_id, 42);
        assert_eq!(info.target_branch(), "feat-a");
        assert_eq!(
            info.web_url().unwrap(),
            "https://dev.azure.com/org/proj/_git/repo/pullrequest/42"
        );
        assert_eq!(vote_summary(&info), "✅ approved (1)");
    }

    #[test]
    fn splice_appends_then_replaces() {
        let appended = splice_block(
            "My PR description",
            "<!-- stack:begin -->v1<!-- stack:end -->",
        );
        assert_eq!(
            appended,
            "My PR description\n\n<!-- stack:begin -->v1<!-- stack:end -->"
        );
        let replaced = splice_block(&appended, "<!-- stack:begin -->v2<!-- stack:end -->");
        assert_eq!(
            replaced,
            "My PR description\n\n<!-- stack:begin -->v2<!-- stack:end -->"
        );
    }

    #[test]
    fn stack_order_excludes_unrelated_roots() {
        let mut state = RepoState::new("develop".to_string(), "origin".to_string());
        state
            .add_branch(crate::state::ManagedBranch::new(
                "feature/a".to_string(),
                "develop".to_string(),
                "base".to_string(),
            ))
            .unwrap();
        state
            .add_branch(crate::state::ManagedBranch::new(
                "feature/b".to_string(),
                "feature/a".to_string(),
                "a".to_string(),
            ))
            .unwrap();
        state
            .add_branch(crate::state::ManagedBranch::new(
                "other/a".to_string(),
                "develop".to_string(),
                "base".to_string(),
            ))
            .unwrap();
        state
            .add_branch(crate::state::ManagedBranch::new(
                "other/b".to_string(),
                "other/a".to_string(),
                "other-a".to_string(),
            ))
            .unwrap();

        let order: Vec<String> = graph::descendants_topo(&state)
            .unwrap()
            .into_iter()
            .map(str::to_string)
            .collect();

        assert_eq!(
            stack_order_for_branch(&state, &order, "feature/b"),
            vec!["feature/a".to_string(), "feature/b".to_string()]
        );
        assert_eq!(
            stack_order_for_branch(&state, &order, "other/a"),
            vec!["other/a".to_string(), "other/b".to_string()]
        );
    }

    #[test]
    fn vote_summary_precedence() {
        let mut info: PrInfo = serde_json::from_str(
            r#"{"pullRequestId":1,"status":"active","targetRefName":"refs/heads/x"}"#,
        )
        .unwrap();
        assert_eq!(vote_summary(&info), "⏳ awaiting review");
        info.reviewers = vec![Reviewer { vote: 10 }, Reviewer { vote: -10 }];
        assert_eq!(vote_summary(&info), "❌ rejected");
        info.is_draft = Some(true);
        assert_eq!(vote_summary(&info), "📝 draft");
    }
}

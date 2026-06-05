mod cleanup;
mod cli;
mod error;
mod git;
mod graph;
mod output;
mod rebase;
mod state;
mod sync;

use anyhow::Result;
use clap::Parser;
use std::env;
use std::io::{self, Write};

use cleanup::cleanup;
use cli::{Cli, Command};
use error::StackError;
use git::GitRepo;
use output::{print_status_json, print_status_text};
use rebase::rebase_branch;
use state::RepoState;
use sync::sync_all;

const DEFAULT_TRUNK: &str = "develop";
const DEFAULT_REMOTE: &str = "origin";

fn main() {
    if let Err(err) = run() {
        eprintln!("Error: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = env::current_dir()?;
    let repo = GitRepo::discover(root)?;

    match cli.command {
        Command::Init(args) => init(&repo, args),
        Command::Status(args) => status(&repo, args.json),
        Command::Create(args) => create(&repo, args),
        Command::Track(args) => track(&repo, args),
        Command::Rebase(args) => {
            rebase_branch(&repo, &args.branch, args.onto.as_deref(), args.dry_run)
        }
        Command::Sync(args) => sync_all(&repo, args.all, args.dry_run),
        Command::MarkMerged(args) => mark_merged(&repo, args.branch.as_deref()),
        Command::Cleanup(args) => cleanup(&repo, args.dry_run),
        Command::Doctor => doctor(&repo),
    }
}

fn init(repo: &GitRepo, args: cli::InitArgs) -> Result<()> {
    let state_path = RepoState::path_in(&repo.root);
    if state_path.exists() {
        anyhow::bail!("stack state already exists at {}", state_path.display());
    }

    let state = RepoState::new(args.trunk, args.remote);
    state.save(&repo.root)?;
    println!("Initialized stack state at {}", state_path.display());
    Ok(())
}

fn status(repo: &GitRepo, json: bool) -> Result<()> {
    let state = RepoState::load(&repo.root)?;
    state.validate_metadata(repo)?;
    let report = output::build_status_report(repo, &state)?;
    if json {
        print_status_json(&report)?;
    } else {
        print_status_text(&state, &report);
    }
    Ok(())
}

fn create(repo: &GitRepo, args: cli::CreateArgs) -> Result<()> {
    let mut state = load_or_init_default_state(repo)?;
    state.validate(repo)?;

    if state.branch(&args.branch).is_some() {
        anyhow::bail!("branch already tracked: {}", args.branch);
    }
    if repo.branch_exists(&args.branch)? {
        anyhow::bail!("branch already exists locally: {}", args.branch);
    }

    let explicit_parent = args.parent.is_some();
    let parent = match args.parent {
        Some(parent) => parent,
        None => repo.current_branch()?,
    };

    if !repo.branch_exists(&parent)? {
        anyhow::bail!("parent branch does not exist: {parent}");
    }

    if !explicit_parent && parent != state.repo.trunk && state.branch(&parent).is_none() {
        let trunk = state.repo.trunk.clone();
        track_branch_with_parent(repo, &mut state, parent.clone(), trunk.clone())?;
        println!("Tracked current branch {parent} with parent {trunk}");
    }

    let recorded_parent_tip = repo.branch_tip(&parent)?;
    repo.create_branch(&args.branch, &parent)?;
    repo.checkout(&args.branch)?;

    state.add_branch(state::ManagedBranch::new(
        args.branch,
        parent,
        recorded_parent_tip,
    ))?;
    state.save(&repo.root)?;
    println!("Created and checked out tracked branch");
    Ok(())
}

fn load_or_init_default_state(repo: &GitRepo) -> Result<RepoState> {
    match RepoState::load(&repo.root) {
        Ok(state) => Ok(state),
        Err(err)
            if err
                .downcast_ref::<StackError>()
                .is_some_and(|e| matches!(e, StackError::NotInitialized)) =>
        {
            let state = RepoState::new(DEFAULT_TRUNK.to_string(), DEFAULT_REMOTE.to_string());
            state.save(&repo.root)?;
            println!(
                "Initialized stack state at {} with trunk '{}' and remote '{}'",
                RepoState::path_in(&repo.root).display(),
                DEFAULT_TRUNK,
                DEFAULT_REMOTE
            );
            Ok(state)
        }
        Err(err) => Err(err),
    }
}

fn track(repo: &GitRepo, args: cli::TrackArgs) -> Result<()> {
    let mut state = load_or_init_default_state(repo)?;
    state.validate(repo)?;

    let branch = match args.branch {
        Some(branch) => branch,
        None => repo.current_branch()?,
    };
    let parent = args.parent.unwrap_or_else(|| state.repo.trunk.clone());

    track_branch_with_parent(repo, &mut state, branch.clone(), parent.clone())?;
    state.save(&repo.root)?;
    println!("Tracked {branch} with parent {parent}");
    Ok(())
}

fn track_branch_with_parent(
    repo: &GitRepo,
    state: &mut RepoState,
    branch: String,
    parent: String,
) -> Result<()> {
    if branch.is_empty() {
        anyhow::bail!("cannot track branch: HEAD is detached");
    }
    if branch == state.repo.trunk {
        anyhow::bail!("trunk branch cannot be tracked");
    }
    if state.branch(&branch).is_some() {
        anyhow::bail!("branch already tracked: {branch}");
    }
    if !repo.branch_exists(&branch)? {
        anyhow::bail!("branch does not exist locally: {branch}");
    }
    if !repo.branch_exists(&parent)? && parent != state.repo.trunk {
        anyhow::bail!("parent branch does not exist locally: {parent}");
    }

    let recorded_parent_tip = repo.branch_tip(&parent)?;
    state.add_branch(state::ManagedBranch::new(
        branch,
        parent,
        recorded_parent_tip,
    ))?;
    Ok(())
}

fn mark_merged(repo: &GitRepo, branch: Option<&str>) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    state.validate_metadata(repo)?;
    let branch = match branch {
        Some(branch) => branch.to_string(),
        None => select_branch_to_mark_merged(&state)?,
    };
    let managed = state
        .branch_mut(&branch)
        .ok_or_else(|| anyhow::anyhow!("branch not tracked: {branch}"))?;
    managed.status = state::BranchStatus::Merged;
    state.save(&repo.root)?;
    println!("Marked {branch} as merged");
    Ok(())
}

fn select_branch_to_mark_merged(state: &RepoState) -> Result<String> {
    let candidates: Vec<&str> = state
        .branches
        .iter()
        .filter(|branch| branch.status == state::BranchStatus::Active)
        .map(|branch| branch.name.as_str())
        .collect();

    if candidates.is_empty() {
        anyhow::bail!("no active tracked branches to mark as merged");
    }

    println!("Select branch to mark as merged:");
    for (index, branch) in candidates.iter().enumerate() {
        println!("  {}. {branch}", index + 1);
    }
    print!("Enter number: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let selection: usize = input
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid selection: expected a number"))?;
    let branch = candidates
        .get(selection.saturating_sub(1))
        .ok_or_else(|| anyhow::anyhow!("invalid selection: {selection}"))?;

    Ok((*branch).to_string())
}

fn doctor(repo: &GitRepo) -> Result<()> {
    let state = RepoState::load(&repo.root)?;
    state.validate(repo)?;
    repo.ensure_branch_exists(&state.repo.trunk)?;
    if repo.is_clean()? {
        println!("Doctor OK: repository and stack state look healthy");
    } else {
        println!("Doctor warning: repository has uncommitted changes");
    }
    Ok(())
}

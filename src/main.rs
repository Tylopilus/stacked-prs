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
        Command::MarkMerged(args) => mark_merged(&repo, &args.branch),
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
    state.validate(repo)?;
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

    let parent = match args.parent {
        Some(parent) => parent,
        None => repo.current_branch()?,
    };

    if !repo.branch_exists(&parent)? {
        anyhow::bail!("parent branch does not exist: {parent}");
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

    if state.branch(&args.branch).is_some() {
        anyhow::bail!("branch already tracked: {}", args.branch);
    }
    if !repo.branch_exists(&args.branch)? {
        anyhow::bail!("branch does not exist locally: {}", args.branch);
    }
    if !repo.branch_exists(&args.parent)? && args.parent != state.repo.trunk {
        anyhow::bail!("parent branch does not exist locally: {}", args.parent);
    }

    let recorded_parent_tip = repo.branch_tip(&args.parent)?;
    state.add_branch(state::ManagedBranch::new(
        args.branch,
        args.parent,
        recorded_parent_tip,
    ))?;
    state.save(&repo.root)?;
    println!("Tracked branch");
    Ok(())
}

fn mark_merged(repo: &GitRepo, branch: &str) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    let managed = state
        .branch_mut(branch)
        .ok_or_else(|| anyhow::anyhow!("branch not tracked: {branch}"))?;
    managed.status = state::BranchStatus::Merged;
    state.save(&repo.root)?;
    println!("Marked {branch} as merged");
    Ok(())
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

mod azure;
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
use clap_complete::generate;
use std::env;
use std::io::{self, Write};

use cleanup::cleanup;
use cli::{Cli, Command};
use error::StackError;
use git::GitRepo;
use output::{print_status_json, print_status_text};
use rebase::rebase_branch;
use state::{PendingReparent, RepoState};
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
    let command = cli.command;
    let command = match command {
        Command::Completions(args) => return completions(args.shell),
        command => command,
    };

    let root = env::current_dir()?;
    let repo = GitRepo::discover(root)?;

    match command {
        Command::Init(args) => init(&repo, args),
        Command::Status(args) => status(&repo, args.json),
        Command::Create(args) => create(&repo, args),
        Command::Track(args) => track(&repo, args),
        Command::Rebase(args) => {
            rebase_branch(&repo, &args.branch, args.onto.as_deref(), args.dry_run)
        }
        Command::Reparent(args) => reparent(&repo, args),
        Command::Sync(args) => sync_all(
            &repo,
            args.all,
            args.from.as_deref(),
            args.continue_sync,
            args.dry_run,
            args.push,
            args.no_pr,
        ),
        Command::MarkMerged(args) => mark_merged(&repo, args.branch.as_deref()),
        Command::Pr(args) => match args.command {
            cli::PrCommand::Create(args) => {
                azure::pr_create(&repo, args.branch, args.title, args.draft)
            }
            cli::PrCommand::Sync(_) => azure::pr_sync(&repo),
        },
        Command::Land(args) => azure::land(&repo, args.branch),
        Command::Cleanup(args) => cleanup(&repo, args.dry_run),
        Command::Doctor => doctor(&repo),
        Command::Completions(_) => unreachable!("handled before repository discovery"),
    }
}

fn completions(shell: clap_complete::Shell) -> Result<()> {
    let mut command = Cli::command_for_completions();
    generate(shell, &mut command, "stacked-prs", &mut io::stdout());
    Ok(())
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
        if let Some(pending) = PendingReparent::load_optional(&repo.root)? {
            println!();
            println!("Pending operation:");
            println!(
                "  reparent {}: {} -> {}",
                pending.branch, pending.old_parent, pending.new_parent
            );
            println!("  continue: stack reparent --continue");
            println!("  abort: stack reparent --abort");
        }
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

fn reparent(repo: &GitRepo, args: cli::ReparentArgs) -> Result<()> {
    if args.continue_reparent {
        return reparent_continue(repo, args);
    }
    if args.abort {
        return reparent_abort(repo, args);
    }
    if PendingReparent::load_optional(&repo.root)?.is_some() {
        anyhow::bail!(
            "pending reparent already exists; run 'stack reparent --continue' or 'stack reparent --abort'"
        );
    }
    repo.ensure_clean()?;
    let state = RepoState::load(&repo.root)?;
    state.validate(repo)?;
    let branch = args.branch.ok_or_else(|| {
        anyhow::anyhow!("missing branch; use 'stack reparent <branch> --parent <branch>'")
    })?;
    let parent = args
        .parent
        .ok_or_else(|| anyhow::anyhow!("missing --parent <branch>"))?;

    if branch == parent {
        anyhow::bail!("branch cannot parent itself: {branch}");
    }
    if branch == state.repo.trunk {
        anyhow::bail!("trunk branch cannot be reparented");
    }
    if state.branch(&branch).is_none() {
        anyhow::bail!("branch not tracked: {branch}");
    }
    if parent != state.repo.trunk && state.branch(&parent).is_none() {
        anyhow::bail!("new parent is not tracked: {parent}");
    }
    if !repo.branch_exists(&parent)? {
        anyhow::bail!("parent branch does not exist locally: {parent}");
    }

    let managed = state
        .branch(&branch)
        .expect("branch existence checked")
        .clone();
    let old_parent = managed.parent;
    let old_parent_tip = managed.recorded_parent_tip;
    let new_parent_tip = repo.branch_tip(&parent)?;

    println!(
        "reparent {}: parent={} old_base={} new_base={}",
        branch,
        parent,
        output::short_sha(&old_parent_tip),
        output::short_sha(&new_parent_tip)
    );

    if args.dry_run {
        return Ok(());
    }

    let pending = PendingReparent::new(
        branch.clone(),
        old_parent,
        parent.clone(),
        old_parent_tip.clone(),
        new_parent_tip.clone(),
    );

    if !args.no_rebase && old_parent_tip != new_parent_tip {
        pending.save(&repo.root)?;
        if let Err(err) = repo.rebase_onto(&new_parent_tip, &old_parent_tip, &branch) {
            println!();
            println!("Rebase stopped before reparent could finish.");
            println!("Resolve conflicts, then run: stack reparent --continue");
            println!("Or abort with: stack reparent --abort");
            return Err(err);
        }
    }

    finalize_reparent(repo, pending)?;
    Ok(())
}

fn reparent_continue(repo: &GitRepo, args: cli::ReparentArgs) -> Result<()> {
    if args.branch.is_some()
        || args.parent.is_some()
        || args.no_rebase
        || args.dry_run
        || args.abort
    {
        anyhow::bail!(
            "--continue cannot be combined with branch, --parent, --no-rebase, --dry-run, or --abort"
        );
    }
    let pending = PendingReparent::load_optional(&repo.root)?
        .ok_or_else(|| anyhow::anyhow!("no pending reparent operation"))?;

    if repo.is_rebase_in_progress()? {
        if let Err(err) = repo.rebase_continue() {
            println!();
            println!("Rebase is still not complete.");
            println!("Resolve conflicts, then run: stack reparent --continue");
            println!("Or abort with: stack reparent --abort");
            return Err(err);
        }
    }

    finalize_reparent(repo, pending)
}

fn reparent_abort(repo: &GitRepo, args: cli::ReparentArgs) -> Result<()> {
    if args.branch.is_some()
        || args.parent.is_some()
        || args.no_rebase
        || args.dry_run
        || args.continue_reparent
    {
        anyhow::bail!(
            "--abort cannot be combined with branch, --parent, --no-rebase, --dry-run, or --continue"
        );
    }
    let pending = PendingReparent::load_optional(&repo.root)?
        .ok_or_else(|| anyhow::anyhow!("no pending reparent operation"))?;

    if repo.is_rebase_in_progress()? {
        repo.rebase_abort()?;
    }
    PendingReparent::clear(&repo.root)?;
    println!("Aborted reparent of {}", pending.branch);
    Ok(())
}

fn finalize_reparent(repo: &GitRepo, pending: PendingReparent) -> Result<()> {
    let mut state = RepoState::load(&repo.root)?;
    let branch = state
        .branch_mut(&pending.branch)
        .ok_or_else(|| anyhow::anyhow!("branch disappeared from state: {}", pending.branch))?;
    branch.parent = pending.new_parent.clone();
    branch.recorded_parent_tip = pending.new_parent_tip;
    state.validate(repo)?;
    state.save(&repo.root)?;
    PendingReparent::clear(&repo.root)?;
    println!("Reparented {}", pending.branch);
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

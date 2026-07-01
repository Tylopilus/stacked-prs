use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser, Debug)]
#[command(name = "stack")]
#[command(about = "Local stacked PR workflow manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    #[command(about = "Initialize local stack metadata for this repository")]
    Init(InitArgs),
    #[command(about = "Show tracked branches, parents, and rebase drift")]
    Status(StatusArgs),
    #[command(about = "Create a new tracked branch stacked on the current or given parent")]
    Create(CreateArgs),
    #[command(about = "Track an existing local branch in the stack")]
    Track(TrackArgs),
    #[command(about = "Rebase one tracked branch onto its current effective parent")]
    Rebase(RebaseArgs),
    #[command(about = "Change a tracked branch's parent in the stack")]
    Reparent(ReparentArgs),
    #[command(about = "Fetch and rebase all tracked branches whose parents moved")]
    Sync(SyncArgs),
    #[command(about = "Mark a tracked branch as squash-merged into trunk (manual override)")]
    MarkMerged(MarkMergedArgs),
    #[command(about = "Manage Azure DevOps pull requests for the stack")]
    Pr(PrArgs),
    #[command(about = "Set auto-complete on the bottom PR of the stack (squash + delete source)")]
    Land(LandArgs),
    #[command(about = "Delete merged leaf branches and prune stale missing branches from state")]
    Cleanup(CleanupArgs),
    #[command(about = "Check repository and stack metadata health")]
    Doctor,
    #[command(about = "Generate shell completions")]
    Completions(CompletionsArgs),
}

impl Cli {
    pub fn command_for_completions() -> clap::Command {
        Self::command()
    }
}

#[derive(Args, Debug)]
pub struct InitArgs {
    #[arg(long, default_value = "develop", help = "Base branch for the stack")]
    pub trunk: String,
    #[arg(long, default_value = "origin", help = "Remote used for fetches")]
    pub remote: String,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(long, help = "Print status as JSON")]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    #[arg(help = "Name of the new branch to create")]
    pub branch: String,
    #[arg(long, help = "Parent branch; defaults to the current branch")]
    pub parent: Option<String>,
}

#[derive(Args, Debug)]
pub struct TrackArgs {
    #[arg(help = "Existing branch to track; defaults to the current branch")]
    pub branch: Option<String>,
    #[arg(long, help = "Parent branch; defaults to the configured trunk")]
    pub parent: Option<String>,
}

#[derive(Args, Debug)]
pub struct RebaseArgs {
    #[arg(help = "Tracked branch to rebase")]
    pub branch: String,
    #[arg(long, help = "Override the branch to rebase onto")]
    pub onto: Option<String>,
    #[arg(long, help = "Print the planned rebase without running it")]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct ReparentArgs {
    #[arg(help = "Tracked branch whose parent should change")]
    pub branch: Option<String>,
    #[arg(long, help = "New parent branch")]
    pub parent: Option<String>,
    #[arg(long, help = "Only update stack metadata; do not run git rebase")]
    pub no_rebase: bool,
    #[arg(long = "continue", help = "Continue a pending conflicted reparent")]
    pub continue_reparent: bool,
    #[arg(long, help = "Abort a pending conflicted reparent")]
    pub abort: bool,
    #[arg(long, help = "Print the planned reparent without changing anything")]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    #[arg(
        long,
        help = "Accepted for compatibility; sync always processes the stack"
    )]
    pub all: bool,
    #[arg(
        long,
        help = "Only sync descendants of this tracked branch; the branch itself is left untouched"
    )]
    pub from: Option<String>,
    #[arg(long = "continue", help = "Continue a conflicted stack sync rebase")]
    pub continue_sync: bool,
    #[arg(long, help = "Print planned rebases without running them")]
    pub dry_run: bool,
    #[arg(
        long,
        help = "Force-push active tracked branches (with lease) to update their PRs"
    )]
    pub push: bool,
    #[arg(long, help = "Skip Azure DevOps PR reconciliation")]
    pub no_pr: bool,
}

#[derive(Args, Debug)]
pub struct PrArgs {
    #[command(subcommand)]
    pub command: PrCommand,
}

#[derive(Subcommand, Debug)]
pub enum PrCommand {
    #[command(about = "Push a tracked branch and open a PR targeting its effective parent")]
    Create(PrCreateArgs),
    #[command(
        about = "Reconcile PRs: auto-mark merged branches, retarget drifted PRs, refresh descriptions"
    )]
    Sync(PrSyncArgs),
}

#[derive(Args, Debug)]
pub struct PrCreateArgs {
    #[arg(help = "Tracked branch; defaults to the current branch")]
    pub branch: Option<String>,
    #[arg(long, help = "PR title; defaults to the branch name")]
    pub title: Option<String>,
    #[arg(long, help = "Create the PR as a draft")]
    pub draft: bool,
}

#[derive(Args, Debug)]
pub struct PrSyncArgs {}

#[derive(Args, Debug)]
pub struct LandArgs {
    #[arg(help = "Branch to land; defaults to the single stack root with a PR targeting trunk")]
    pub branch: Option<String>,
}

#[derive(Args, Debug)]
pub struct MarkMergedArgs {
    #[arg(help = "Tracked branch that was squash-merged; prompts when omitted")]
    pub branch: Option<String>,
}

#[derive(Args, Debug)]
pub struct CleanupArgs {
    #[arg(long, help = "Print cleanup candidates without deleting or pruning")]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    #[arg(help = "Shell to generate completions for")]
    pub shell: Shell,
}

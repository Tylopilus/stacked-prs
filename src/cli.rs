use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "stack")]
#[command(about = "Local stacked PR workflow manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Init(InitArgs),
    Status(StatusArgs),
    Create(CreateArgs),
    Track(TrackArgs),
    Rebase(RebaseArgs),
    Sync(SyncArgs),
    MarkMerged(MarkMergedArgs),
    Cleanup(CleanupArgs),
    Doctor,
}

#[derive(Args, Debug)]
pub struct InitArgs {
    #[arg(long, default_value = "develop")]
    pub trunk: String,
    #[arg(long, default_value = "origin")]
    pub remote: String,
}

#[derive(Args, Debug)]
pub struct StatusArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    pub branch: String,
    #[arg(long)]
    pub parent: Option<String>,
}

#[derive(Args, Debug)]
pub struct TrackArgs {
    pub branch: String,
    #[arg(long)]
    pub parent: String,
}

#[derive(Args, Debug)]
pub struct RebaseArgs {
    pub branch: String,
    #[arg(long)]
    pub onto: Option<String>,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct SyncArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Debug)]
pub struct MarkMergedArgs {
    pub branch: String,
}

#[derive(Args, Debug)]
pub struct CleanupArgs {
    #[arg(long)]
    pub dry_run: bool,
}

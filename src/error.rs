use thiserror::Error;

#[derive(Debug, Error)]
pub enum StackError {
    #[error("state file not initialized")]
    NotInitialized,
    #[error("branch already tracked: {0}")]
    BranchAlreadyTracked(String),
    #[error("invalid stack graph: {0}")]
    InvalidGraph(String),
    #[error("working tree is dirty")]
    DirtyWorktree,
}

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use tempfile::TempDir;

fn setup_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init", "-b", "develop"]);
    run_git(dir.path(), &["config", "user.name", "Test User"]);
    run_git(dir.path(), &["config", "user.email", "test@example.com"]);
    fs::write(dir.path().join(".gitignore"), "/.stacked-prs/\n").unwrap();
    fs::write(dir.path().join("file.txt"), "base\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "base"]);
    dir
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git command failed: {:?}", args);
}

fn run_stack(repo: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("stacked-prs"));
    cmd.args(args).current_dir(repo).assert()
}

fn write_and_commit(repo: &Path, path: &str, content: &str, message: &str) {
    fs::write(repo.join(path), content).unwrap();
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", message]);
}

#[test]
fn init_creates_state_file() {
    let repo = setup_repo();
    run_stack(repo.path(), &["init"]).success();
    assert!(repo.path().join(".stacked-prs/state.json").exists());
}

#[test]
fn create_tracks_branch_and_checks_it_out() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();

    let head = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), "feature/a");

    let state = fs::read_to_string(repo.path().join(".stacked-prs/state.json")).unwrap();
    assert!(state.contains("feature/a"));
}

#[test]
fn track_auto_initializes_repo_state() {
    let repo = setup_repo();
    run_git(repo.path(), &["branch", "feature/a", "develop"]);

    run_stack(repo.path(), &["track", "feature/a", "--parent", "develop"]).success();

    let state = fs::read_to_string(repo.path().join(".stacked-prs/state.json")).unwrap();
    assert!(state.contains("feature/a"));
}

#[test]
fn dirty_worktree_blocks_rebase() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    fs::write(repo.path().join("dirty.txt"), "oops\n").unwrap();

    run_stack(repo.path(), &["rebase", "feature/a"]).failure();
}

#[test]
fn cleanup_deletes_merged_leaf_branch() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    run_git(repo.path(), &["checkout", "develop"]);
    run_stack(repo.path(), &["mark-merged", "feature/a"]).success();
    run_stack(repo.path(), &["cleanup"]).success();

    let output = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", "refs/heads/feature/a"])
        .current_dir(repo.path())
        .status()
        .unwrap();
    assert!(!output.success());
}

#[test]
fn sync_rebases_child_after_parent_squash_merge() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    write_and_commit(repo.path(), "a.txt", "from a\n", "feature a change");

    run_stack(repo.path(), &["create", "feature/b"]).success();
    write_and_commit(repo.path(), "b.txt", "from b\n", "feature b change");

    run_git(repo.path(), &["checkout", "develop"]);
    run_git(repo.path(), &["merge", "--squash", "feature/a"]);
    run_git(repo.path(), &["commit", "-m", "squash feature a"]);

    run_stack(repo.path(), &["mark-merged", "feature/a"]).success();

    run_stack(repo.path(), &["sync", "--all"]).success();

    let diff = Command::new("git")
        .args(["diff", "--name-only", "develop...feature/b"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let diff = String::from_utf8(diff.stdout).unwrap();
    assert!(diff.contains("b.txt"));
    assert!(!diff.contains("a.txt"));
}

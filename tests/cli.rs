use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::prelude::*;
use serde_json::{Value, json};
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

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(output.status.success(), "git command failed: {:?}", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn run_stack(repo: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("stacked-prs"));
    cmd.args(args).current_dir(repo).assert()
}

fn run_stack_with_stdin(repo: &Path, args: &[&str], stdin: &str) -> assert_cmd::assert::Assert {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("stacked-prs");
    cmd.args(args).current_dir(repo).write_stdin(stdin).assert()
}

fn write_and_commit(repo: &Path, path: &str, content: &str, message: &str) {
    fs::write(repo.join(path), content).unwrap();
    run_git(repo, &["add", "."]);
    run_git(repo, &["commit", "-m", message]);
}

fn branch_parent(repo: &Path, branch_name: &str) -> String {
    let state = fs::read_to_string(repo.join(".stacked-prs/state.json")).unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    state["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["name"] == branch_name)
        .unwrap_or_else(|| panic!("branch not tracked: {branch_name}"))["parent"]
        .as_str()
        .unwrap()
        .to_string()
}

fn branch_status(repo: &Path, branch_name: &str) -> String {
    let state = fs::read_to_string(repo.join(".stacked-prs/state.json")).unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    state["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["name"] == branch_name)
        .unwrap_or_else(|| panic!("branch not tracked: {branch_name}"))["status"]
        .as_str()
        .unwrap()
        .to_string()
}

fn tracked_branch_count(repo: &Path) -> usize {
    let state = fs::read_to_string(repo.join(".stacked-prs/state.json")).unwrap();
    let state: Value = serde_json::from_str(&state).unwrap();
    state["branches"].as_array().unwrap().len()
}

fn set_branch_pr(repo: &Path, branch_name: &str, pr_id: &str, target: &str) {
    let path = repo.join(".stacked-prs/state.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let branch = state["branches"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|branch| branch["name"] == branch_name)
        .unwrap_or_else(|| panic!("branch not tracked: {branch_name}"));
    branch["pr"] = json!({
        "provider": "azure-devops",
        "id": pr_id,
        "url": format!("https://dev.azure.test/pr/{pr_id}"),
        "target_branch": target,
    });
    fs::write(path, serde_json::to_string_pretty(&state).unwrap()).unwrap();
}

fn pending_operation_exists(repo: &Path) -> bool {
    repo.join(".stacked-prs/pending.json").exists()
}

#[test]
fn init_creates_state_file() {
    let repo = setup_repo();
    run_stack(repo.path(), &["init"]).success();
    assert!(repo.path().join(".stacked-prs/state.json").exists());
}

#[test]
fn help_describes_commands() {
    let repo = setup_repo();
    run_stack(repo.path(), &["--help"])
        .success()
        .stdout(predicates::str::contains(
            "Track an existing local branch in the stack",
        ))
        .stdout(predicates::str::contains(
            "Delete merged leaf branches and prune stale missing branches from state",
        ));
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
fn track_defaults_to_current_branch_and_trunk_parent() {
    let repo = setup_repo();
    run_git(repo.path(), &["checkout", "-b", "feature/a", "develop"]);

    run_stack(repo.path(), &["track"]).success();

    assert_eq!(branch_parent(repo.path(), "feature/a"), "develop");
}

#[test]
fn create_auto_tracks_untracked_current_branch() {
    let repo = setup_repo();
    run_git(repo.path(), &["checkout", "-b", "feature/a", "develop"]);

    run_stack(repo.path(), &["create", "feature/b"]).success();

    assert_eq!(branch_parent(repo.path(), "feature/a"), "develop");
    assert_eq!(branch_parent(repo.path(), "feature/b"), "feature/a");
}

#[test]
fn pr_list_outputs_tracked_prs_as_json() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    run_stack(repo.path(), &["create", "feature/b"]).success();
    set_branch_pr(repo.path(), "feature/a", "101", "develop");
    set_branch_pr(repo.path(), "feature/b", "102", "feature/a");

    let output = run_stack(repo.path(), &["pr", "list", "--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    let output: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(output.as_array().unwrap().len(), 2);
    assert_eq!(output[0]["branch"], "feature/a");
    assert_eq!(output[0]["effective_parent"], "develop");
    assert_eq!(output[0]["pr"]["id"], "101");
    assert_eq!(output[1]["branch"], "feature/b");
    assert_eq!(output[1]["effective_parent"], "feature/a");
    assert_eq!(output[1]["pr"]["id"], "102");
}

#[test]
fn pr_show_accepts_pr_id_and_includes_stack() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    run_stack(repo.path(), &["create", "feature/b"]).success();
    set_branch_pr(repo.path(), "feature/a", "101", "develop");
    set_branch_pr(repo.path(), "feature/b", "102", "feature/a");

    let output = run_stack(repo.path(), &["pr", "show", "!102", "--json"])
        .success()
        .get_output()
        .stdout
        .clone();
    let output: Value = serde_json::from_slice(&output).unwrap();

    assert_eq!(output["pr"]["branch"], "feature/b");
    assert_eq!(output["pr"]["pr"]["id"], "102");
    assert_eq!(output["stack"].as_array().unwrap().len(), 2);
    assert_eq!(output["stack"][0]["branch"], "feature/a");
    assert_eq!(output["stack"][0]["pr_id"], "101");
    assert_eq!(output["stack"][1]["branch"], "feature/b");
    assert_eq!(output["stack"][1]["current"], true);
}

#[test]
fn reparent_updates_state_after_manual_rebase() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    write_and_commit(repo.path(), "a.txt", "from a\n", "feature a change");

    run_git(repo.path(), &["checkout", "develop"]);
    run_stack(repo.path(), &["create", "feature/base"]).success();
    write_and_commit(repo.path(), "base.txt", "from base\n", "base change");

    run_git(repo.path(), &["checkout", "feature/a"]);
    run_git(repo.path(), &["rebase", "feature/base"]);

    run_stack(
        repo.path(),
        &[
            "reparent",
            "feature/a",
            "--parent",
            "feature/base",
            "--no-rebase",
        ],
    )
    .success();

    assert_eq!(branch_parent(repo.path(), "feature/a"), "feature/base");
    run_stack(repo.path(), &["status"])
        .success()
        .stdout(predicates::str::contains("drift: up_to_date"));
}

#[test]
fn conflicted_reparent_can_be_aborted() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    write_and_commit(repo.path(), "file.txt", "feature a\n", "feature a change");

    run_git(repo.path(), &["checkout", "develop"]);
    run_stack(repo.path(), &["create", "feature/base"]).success();
    write_and_commit(repo.path(), "file.txt", "feature base\n", "base change");

    run_stack(
        repo.path(),
        &["reparent", "feature/a", "--parent", "feature/base"],
    )
    .failure()
    .stdout(predicates::str::contains("stack reparent --continue"))
    .stdout(predicates::str::contains("stack reparent --abort"));

    assert!(pending_operation_exists(repo.path()));
    assert_eq!(branch_parent(repo.path(), "feature/a"), "develop");

    run_stack(repo.path(), &["status"])
        .success()
        .stdout(predicates::str::contains("Pending operation:"));

    run_stack(repo.path(), &["reparent", "--abort"]).success();

    assert!(!pending_operation_exists(repo.path()));
    assert_eq!(branch_parent(repo.path(), "feature/a"), "develop");
}

#[test]
fn status_reports_missing_tracked_branch() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    run_git(repo.path(), &["checkout", "develop"]);
    run_git(repo.path(), &["branch", "-D", "feature/a"]);

    run_stack(repo.path(), &["status"])
        .success()
        .stdout(predicates::str::contains("feature/a [missing]"));
}

#[test]
fn cleanup_prunes_missing_tracked_branches() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    run_stack(repo.path(), &["create", "feature/b"]).success();
    run_git(repo.path(), &["checkout", "develop"]);
    run_git(repo.path(), &["branch", "-D", "feature/b"]);
    run_git(repo.path(), &["branch", "-D", "feature/a"]);

    run_stack(repo.path(), &["cleanup"]).success();

    assert_eq!(tracked_branch_count(repo.path()), 0);
}

#[test]
fn dirty_worktree_blocks_rebase() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    write_and_commit(repo.path(), "tracked.txt", "tracked\n", "tracked file");
    fs::write(repo.path().join("tracked.txt"), "oops\n").unwrap();

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
fn mark_merged_prompts_for_branch_when_omitted() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    run_stack(repo.path(), &["create", "feature/b"]).success();

    run_stack_with_stdin(repo.path(), &["mark-merged"], "2\n")
        .success()
        .stdout(predicates::str::contains("Select branch to mark as merged"))
        .stdout(predicates::str::contains("Marked feature/b as merged"));

    assert_eq!(branch_status(repo.path(), "feature/a"), "active");
    assert_eq!(branch_status(repo.path(), "feature/b"), "merged");
}

#[test]
fn mark_merged_allows_missing_local_branch() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    run_git(repo.path(), &["checkout", "develop"]);
    run_git(repo.path(), &["branch", "-D", "feature/a"]);

    run_stack(repo.path(), &["mark-merged", "feature/a"]).success();

    assert_eq!(branch_status(repo.path(), "feature/a"), "merged");
}

#[test]
fn validation_allows_tracking_after_merged_branch_refs_are_deleted() {
    let repo = setup_repo();
    run_git(repo.path(), &["branch", "feature/ancestor"]);
    run_stack(
        repo.path(),
        &["create", "feature/merged", "--parent", "feature/ancestor"],
    )
    .success();
    run_stack(repo.path(), &["mark-merged", "feature/merged"]).success();
    run_git(repo.path(), &["checkout", "develop"]);
    run_git(
        repo.path(),
        &["branch", "-D", "feature/merged", "feature/ancestor"],
    );
    run_git(repo.path(), &["branch", "feature/configs"]);

    run_stack(repo.path(), &["doctor"]).success();
    run_stack(
        repo.path(),
        &["track", "feature/configs", "--parent", "develop"],
    )
    .success();

    assert_eq!(branch_parent(repo.path(), "feature/configs"), "develop");
}

#[test]
fn validation_allows_sync_after_merged_branch_refs_are_deleted() {
    let repo = setup_repo();
    run_git(repo.path(), &["branch", "feature/ancestor"]);
    run_stack(
        repo.path(),
        &["create", "feature/merged", "--parent", "feature/ancestor"],
    )
    .success();
    write_and_commit(repo.path(), "a.txt", "from a\n", "feature a change");
    run_stack(repo.path(), &["create", "feature/child"]).success();
    write_and_commit(repo.path(), "b.txt", "from b\n", "feature b change");

    run_git(repo.path(), &["checkout", "develop"]);
    run_git(repo.path(), &["merge", "--squash", "feature/merged"]);
    run_git(repo.path(), &["commit", "-m", "squash feature a"]);
    run_stack(repo.path(), &["mark-merged", "feature/merged"]).success();
    run_git(
        repo.path(),
        &["branch", "-D", "feature/merged", "feature/ancestor"],
    );
    run_git(repo.path(), &["checkout", "feature/child"]);

    run_stack(repo.path(), &["sync", "--all", "--no-pr"]).success();
    run_stack(repo.path(), &["doctor"]).success();

    assert_eq!(
        git_output(repo.path(), &["merge-base", "develop", "feature/child"]),
        git_output(repo.path(), &["rev-parse", "develop"])
    );
    assert_eq!(
        git_output(
            repo.path(),
            &["diff", "--name-only", "develop...feature/child"],
        ),
        "b.txt"
    );
    run_stack(repo.path(), &["sync", "--dry-run", "--no-pr"])
        .success()
        .stdout(predicates::str::contains(
            "All tracked branches are up to date",
        ));
}

#[test]
fn validation_rejects_missing_active_branch() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/active"]).success();
    run_git(repo.path(), &["checkout", "develop"]);
    run_git(repo.path(), &["branch", "-D", "feature/active"]);

    run_stack(repo.path(), &["doctor"])
        .failure()
        .stderr(predicates::str::contains(
            "branch does not exist locally: feature/active",
        ));
}

#[test]
fn validation_rejects_missing_active_parent() {
    let repo = setup_repo();
    run_git(repo.path(), &["branch", "feature/parent"]);
    run_stack(
        repo.path(),
        &["create", "feature/active", "--parent", "feature/parent"],
    )
    .success();
    run_git(repo.path(), &["branch", "-D", "feature/parent"]);

    run_stack(repo.path(), &["doctor"])
        .failure()
        .stderr(predicates::str::contains(
            "branch does not exist locally: feature/parent",
        ));
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

#[test]
fn sync_rebases_stack_onto_fetched_trunk_after_squash_merge() {
    for remote_name in ["origin", "upstream"] {
        let repo = setup_repo();
        let remote = TempDir::new().unwrap();
        run_git(remote.path(), &["init", "--bare"]);
        run_git(
            repo.path(),
            &[
                "remote",
                "add",
                remote_name,
                remote.path().to_str().unwrap(),
            ],
        );
        run_git(repo.path(), &["push", "-u", remote_name, "develop"]);
        run_stack(repo.path(), &["init", "--remote", remote_name]).success();
        let local_trunk = git_output(repo.path(), &["rev-parse", "develop"]);

        run_stack(repo.path(), &["create", "feature/a"]).success();
        write_and_commit(repo.path(), "file.txt", "from a\n", "feature a change");
        run_stack(repo.path(), &["create", "feature/b"]).success();
        write_and_commit(repo.path(), "file.txt", "from b\n", "feature b change");
        run_stack(repo.path(), &["create", "feature/c"]).success();
        write_and_commit(repo.path(), "c.txt", "from c\n", "feature c change");
        run_git(
            repo.path(),
            &["push", remote_name, "feature/a", "feature/b", "feature/c"],
        );

        let merger = setup_repo();
        run_git(
            merger.path(),
            &[
                "remote",
                "add",
                remote_name,
                remote.path().to_str().unwrap(),
            ],
        );
        run_git(merger.path(), &["fetch", remote_name]);
        run_git(
            merger.path(),
            &[
                "checkout",
                "-b",
                "landed",
                &format!("refs/remotes/{remote_name}/develop"),
            ],
        );
        run_git(
            merger.path(),
            &[
                "merge",
                "--squash",
                &format!("refs/remotes/{remote_name}/feature/a"),
            ],
        );
        run_git(merger.path(), &["commit", "-m", "squash feature a"]);
        run_git(merger.path(), &["push", remote_name, "HEAD:develop"]);
        run_stack(repo.path(), &["mark-merged", "feature/a"]).success();

        run_stack(repo.path(), &["sync", "--all", "--push", "--no-pr"]).success();

        let fetched_trunk = git_output(
            repo.path(),
            &["rev-parse", &format!("refs/remotes/{remote_name}/develop")],
        );
        let rebased_parent = git_output(repo.path(), &["rev-parse", "feature/b"]);
        assert_eq!(
            local_trunk,
            git_output(repo.path(), &["rev-parse", "develop"])
        );
        assert_eq!(
            fetched_trunk,
            git_output(repo.path(), &["merge-base", &fetched_trunk, "feature/b"])
        );
        assert_eq!(
            rebased_parent,
            git_output(repo.path(), &["merge-base", "feature/b", "feature/c"])
        );
        assert_eq!(
            "feature b change",
            git_output(
                repo.path(),
                &["log", "--format=%s", &format!("{fetched_trunk}..feature/b")],
            )
        );
        assert_eq!(
            "feature c change",
            git_output(repo.path(), &["log", "--format=%s", "feature/b..feature/c"])
        );
        for branch in ["feature/b", "feature/c"] {
            assert_eq!(
                git_output(repo.path(), &["rev-parse", branch]),
                git_output(
                    remote.path(),
                    &[
                        "--git-dir",
                        remote.path().to_str().unwrap(),
                        "rev-parse",
                        &format!("refs/heads/{branch}"),
                    ],
                )
            );
        }

        let state: Value = serde_json::from_str(
            &fs::read_to_string(repo.path().join(".stacked-prs/state.json")).unwrap(),
        )
        .unwrap();
        for (name, expected_tip) in [
            ("feature/b", fetched_trunk.as_str()),
            ("feature/c", rebased_parent.as_str()),
        ] {
            let branch = state["branches"]
                .as_array()
                .unwrap()
                .iter()
                .find(|branch| branch["name"] == name)
                .unwrap();
            assert_eq!(
                branch["recorded_parent_tip"].as_str().unwrap(),
                expected_tip
            );
        }
        run_stack(repo.path(), &["sync", "--all", "--no-pr"])
            .success()
            .stdout(predicates::str::contains(
                "All tracked branches are up to date",
            ));
    }
}

#[test]
fn sync_from_rebases_only_descendants() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    write_and_commit(repo.path(), "a.txt", "from a\n", "feature a change");

    run_stack(repo.path(), &["create", "feature/b"]).success();
    write_and_commit(repo.path(), "b.txt", "from b\n", "feature b change");

    run_stack(repo.path(), &["create", "feature/c"]).success();
    write_and_commit(repo.path(), "c.txt", "from c\n", "feature c change");

    let feature_a_before = git_output(repo.path(), &["rev-parse", "feature/a"]);

    run_git(repo.path(), &["checkout", "feature/b"]);
    write_and_commit(
        repo.path(),
        "b2.txt",
        "from b2\n",
        "feature b second change",
    );

    run_stack(repo.path(), &["sync", "--from", "feature/b"])
        .success()
        .stdout(predicates::str::contains("rebase feature/c"));

    assert_eq!(
        feature_a_before,
        git_output(repo.path(), &["rev-parse", "feature/a"])
    );
    assert_eq!(
        git_output(repo.path(), &["rev-parse", "feature/b"]),
        git_output(repo.path(), &["merge-base", "feature/b", "feature/c"])
    );
}

#[test]
fn sync_records_externally_completed_rebase() {
    let repo = setup_repo();
    run_stack(repo.path(), &["create", "feature/a"]).success();
    write_and_commit(repo.path(), "a.txt", "from a\n", "feature a change");

    run_stack(repo.path(), &["create", "feature/b"]).success();
    write_and_commit(repo.path(), "b.txt", "from b\n", "feature b change");

    run_git(repo.path(), &["checkout", "feature/a"]);
    write_and_commit(
        repo.path(),
        "a2.txt",
        "from a2\n",
        "feature a second change",
    );

    run_git(repo.path(), &["checkout", "feature/b"]);
    run_git(repo.path(), &["rebase", "feature/a"]);

    run_stack(repo.path(), &["sync", "--from", "feature/a"])
        .success()
        .stdout(predicates::str::contains(
            "Recorded completed rebase for feature/b",
        ));

    run_stack(repo.path(), &["sync", "--from", "feature/a"])
        .success()
        .stdout(predicates::str::contains(
            "All descendants of feature/a are up to date",
        ));
}

#[test]
fn completions_generate_sync_from_flag() {
    run_stack(Path::new("/"), &["completions", "zsh"])
        .success()
        .stdout(predicates::str::contains("--from"));
}

#[test]
fn sync_push_pushes_active_branch_without_rebase_plan() {
    let repo = setup_repo();
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    run_git(
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    run_git(repo.path(), &["push", "-u", "origin", "develop"]);

    run_stack(repo.path(), &["create", "feature/a"]).success();
    write_and_commit(repo.path(), "a.txt", "from a\n", "feature a change");

    run_stack(repo.path(), &["sync", "--all", "--push", "--no-pr"])
        .success()
        .stdout(predicates::str::contains(
            "Pushed feature/a (force-with-lease)",
        ));

    assert_eq!(
        git_output(repo.path(), &["rev-parse", "feature/a"]),
        git_output(
            remote.path(),
            &[
                "--git-dir",
                remote.path().to_str().unwrap(),
                "rev-parse",
                "refs/heads/feature/a",
            ],
        ),
    );
}

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
    assert!(
        output.status.success(),
        "git command failed: {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
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

#[cfg(unix)]
mod pr_metadata {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const BRANCH: &str = "feat/1470-metadata";
    const BODY: &str = "## TL;DR\nGenerate useful PR metadata.\n\n## Validation\nNot run by the metadata generator.";

    struct Fixture {
        repo: TempDir,
        remote: TempDir,
        tools: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let repo = setup_repo();
            let remote = TempDir::new().unwrap();
            let tools = TempDir::new().unwrap();
            run_git(remote.path(), &["init", "--bare"]);
            run_git(
                repo.path(),
                &["remote", "add", "origin", remote.path().to_str().unwrap()],
            );
            run_git(repo.path(), &["push", "origin", "develop"]);
            run_stack(repo.path(), &["create", BRANCH]).success();
            fs::create_dir(repo.path().join(".azuredevops")).unwrap();
            fs::write(
                repo.path().join(".azuredevops/pull_request_template.md"),
                "## TL;DR\n<!-- Explain -->\n\n## Validation\n<!-- Actual checks -->",
            )
            .unwrap();
            write_and_commit(
                repo.path(),
                "change.txt",
                "parent feature content\n",
                "feat: #1470 generate PR metadata",
            );
            let fixture = Self {
                repo,
                remote,
                tools,
            };
            fixture.response("feat: #1470 generate PR metadata", BODY);
            fixture.script(
                "copilot",
                "#!/bin/sh\nprintf 'copilot\\n' >> \"$COMMAND_LOG\"\nprintf '%s\\n' \"$@\" > \"$COPILOT_ARGS\"\ncat \"$COPILOT_RESPONSE\"\n",
            );
            fixture.script(
                "az",
                "#!/bin/sh\nprintf 'az\\n' >> \"$COMMAND_LOG\"\nprintf '%s\\n' \"$@\" >> \"$AZ_ARGS\"\ncat \"$AZ_RESPONSE\"\n",
            );
            fs::write(
                fixture.tools.path().join("azure.json"),
                serde_json::to_vec(&json!({
                    "pullRequestId": 201,
                    "status": "active",
                    "targetRefName": "refs/heads/develop",
                    "description": BODY,
                    "repository": {"webUrl": "https://dev.azure.test/repo"}
                }))
                .unwrap(),
            )
            .unwrap();
            fixture
        }

        fn script(&self, name: &str, content: &str) {
            let path = self.tools.path().join(name);
            fs::write(&path, content).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn response(&self, title: &str, description: &str) {
            fs::write(
                self.tools.path().join("copilot.json"),
                serde_json::to_vec(&json!({"title": title, "description": description})).unwrap(),
            )
            .unwrap();
        }

        fn template(&self, path: &str, content: &str) {
            fs::create_dir_all(self.repo.path().join(path).parent().unwrap()).unwrap();
            write_and_commit(
                self.repo.path(),
                path,
                content,
                "docs: #1470 configure PR template",
            );
        }

        fn command(&self, args: &[&str]) -> Command {
            let mut paths = vec![self.tools.path().to_path_buf()];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
            let mut command = Command::new(assert_cmd::cargo::cargo_bin!("stacked-prs"));
            command
                .args(args)
                .current_dir(self.repo.path())
                .env("PATH", std::env::join_paths(paths).unwrap())
                .env("COMMAND_LOG", self.tools.path().join("commands"))
                .env("COPILOT_ARGS", self.tools.path().join("copilot-args"))
                .env("COPILOT_RESPONSE", self.tools.path().join("copilot.json"))
                .env("AZ_ARGS", self.tools.path().join("azure-args"))
                .env("AZ_RESPONSE", self.tools.path().join("azure.json"));
            command
        }

        fn no_remote_effects(&self) {
            assert!(
                !git_output(
                    self.repo.path(),
                    &["ls-remote", self.remote.path().to_str().unwrap()]
                )
                .contains(BRANCH),
                "source branch was pushed before metadata was accepted"
            );
            assert!(!self.tools.path().join("azure-args").exists());
            let state: Value = serde_json::from_str(
                &fs::read_to_string(self.repo.path().join(".stacked-prs/state.json")).unwrap(),
            )
            .unwrap();
            assert!(state["branches"][0]["pr"].is_null());
        }

        fn prompt_data(&self) -> Value {
            let args = fs::read_to_string(self.tools.path().join("copilot-args")).unwrap();
            let data = args
                .split_once("INPUT DATA:\n")
                .unwrap()
                .1
                .lines()
                .next()
                .unwrap();
            serde_json::from_str(data).unwrap()
        }
    }

    #[test]
    fn creates_pr_with_generated_metadata_and_preserves_description_in_stack_block() {
        let fixture = Fixture::new();
        fixture
            .command(&["pr", "create", "--yes"])
            .assert()
            .success();

        let args = fs::read_to_string(fixture.tools.path().join("azure-args")).unwrap();
        assert!(args.contains("--title\nfeat: #1470 generate PR metadata\n"));
        assert!(args.contains(&format!("--description\n{BODY}\n")));
        assert!(args.contains("--work-items\n1470\n"));
        assert!(args.contains(&format!("--description={BODY}\n\n<!-- stack:begin -->")));
        let log = fs::read_to_string(fixture.tools.path().join("commands")).unwrap();
        assert!(log.starts_with("copilot\naz\n"));
        let copilot_args = fs::read_to_string(fixture.tools.path().join("copilot-args")).unwrap();
        assert!(copilot_args.contains("--available-tools=\n"));
        assert!(!copilot_args.contains("--allow-all"));
        assert!(
            git_output(
                fixture.repo.path(),
                &["ls-remote", fixture.remote.path().to_str().unwrap()]
            )
            .contains(BRANCH)
        );
        let state: Value = serde_json::from_str(
            &fs::read_to_string(fixture.repo.path().join(".stacked-prs/state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(state["branches"][0]["pr"]["id"], "201");
    }

    #[test]
    fn previews_metadata_without_push_or_azure_calls() {
        let fixture = Fixture::new();
        let state_path = fixture.repo.path().join(".stacked-prs/state.json");
        let state_before = fs::read(&state_path).unwrap();
        let output = fixture
            .command(&["pr", "create", "--preview"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let metadata: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(metadata["title"], "feat: #1470 generate PR metadata");
        assert_eq!(metadata["description"], BODY);
        assert_eq!(fs::read(state_path).unwrap(), state_before);
        fixture.no_remote_effects();
    }

    #[test]
    fn rejects_bad_generated_metadata_without_remote_effects() {
        for (title, description) in [
            (BRANCH, BODY),
            ("feat: #999 wrong PBI", BODY),
            ("feat: #1470 summary", ""),
            (
                "feat: #1470 summary",
                "## TL;DR\nSummary without validation.",
            ),
            (
                "feat: #1470 summary",
                "## TL;DR\nTODO\n## Validation\nNot run.",
            ),
            (
                "feat: #1470 summary",
                "<!-- stack:begin -->fake<!-- stack:end -->",
            ),
        ] {
            let fixture = Fixture::new();
            fixture.response(title, description);
            fixture
                .command(&["pr", "create", "--yes"])
                .assert()
                .failure();
            fixture.no_remote_effects();
        }
    }

    #[test]
    fn rejects_non_json_and_failed_copilot_without_remote_effects() {
        let fixture = Fixture::new();
        fs::write(fixture.tools.path().join("copilot.json"), "not JSON").unwrap();
        fixture
            .command(&["pr", "create", "--yes"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("Copilot did not return valid"));
        fixture.no_remote_effects();

        fixture.script(
            "copilot",
            "#!/bin/sh\necho 'authentication required' >&2\nexit 1\n",
        );
        fixture
            .command(&["pr", "create", "--yes"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("authentication required"));
        fixture.no_remote_effects();
    }

    #[test]
    fn manual_metadata_does_not_invoke_copilot() {
        let fixture = Fixture::new();
        let body_file = fixture.tools.path().join("description.md");
        fs::write(&body_file, BODY).unwrap();
        fixture
            .command(&[
                "pr",
                "create",
                "--no-ai",
                "--title",
                "docs: #1470 explain PR creation",
                "--description-file",
                body_file.to_str().unwrap(),
                "--yes",
            ])
            .assert()
            .success();
        assert!(!fixture.tools.path().join("copilot-args").exists());
        let args = fs::read_to_string(fixture.tools.path().join("azure-args")).unwrap();
        assert!(args.contains("docs: #1470 explain PR creation"));
        assert!(args.contains(BODY));
    }

    #[test]
    fn requires_complete_manual_metadata_and_confirmation() {
        let fixture = Fixture::new();
        fixture
            .command(&[
                "pr",
                "create",
                "--no-ai",
                "--title",
                "docs: #1470 describe changes",
            ])
            .assert()
            .failure()
            .stderr(predicates::str::contains("--no-ai requires both"));
        fixture.no_remote_effects();
        fixture
            .command(&["pr", "create"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("requires confirmation"));
        fixture.no_remote_effects();
    }

    #[test]
    fn uses_requested_committed_branch_not_the_current_worktree() {
        let fixture = Fixture::new();
        run_git(fixture.repo.path(), &["checkout", "develop"]);
        fs::write(
            fixture.repo.path().join("file.txt"),
            "uncommitted unrelated text\n",
        )
        .unwrap();
        fixture
            .command(&["pr", "create", BRANCH, "--preview"])
            .assert()
            .success();
        let data = fixture.prompt_data();
        assert_eq!(data["branch"], BRANCH);
        assert_eq!(data["target"], "develop");
        assert!(
            data["diff"]
                .as_str()
                .unwrap()
                .contains("parent feature content")
        );
        assert!(
            !data["diff"]
                .as_str()
                .unwrap()
                .contains("uncommitted unrelated text")
        );
        assert!(data["template"].as_str().unwrap().contains("## Validation"));
        fixture.no_remote_effects();
    }

    #[test]
    fn stacked_generation_excludes_parent_changes() {
        let fixture = Fixture::new();
        run_stack(fixture.repo.path(), &["create", "feat/1471-child"]).success();
        write_and_commit(
            fixture.repo.path(),
            "child.txt",
            "only child changes\n",
            "feat: #1471 child change",
        );
        fixture.response("feat: #1471 add child behavior", BODY);
        fixture
            .command(&["pr", "create", "--preview"])
            .assert()
            .success();
        let data = fixture.prompt_data();
        assert_eq!(data["target"], BRANCH);
        assert_eq!(data["primary_pbi"], "1471");
        assert_eq!(data["changed_paths"], "child.txt");
        assert!(
            !data["diff"]
                .as_str()
                .unwrap()
                .contains("parent feature content")
        );
        fixture.no_remote_effects();
    }

    #[test]
    fn explicit_template_and_title_overrides_are_honored() {
        let fixture = Fixture::new();
        let template = fixture.tools.path().join("custom.md");
        fs::write(&template, "## Custom\n<!-- Describe -->").unwrap();
        fixture.response(
            "feat: #1470 generated title",
            "## Custom\nCustom explanation.",
        );
        let output = fixture
            .command(&[
                "pr",
                "create",
                "--preview",
                "--template",
                template.to_str().unwrap(),
                "--title",
                "fix: #1470 use explicit title",
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let data: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(data["title"], "fix: #1470 use explicit title");
        assert_eq!(
            fixture.prompt_data()["template"],
            "## Custom\n<!-- Describe -->"
        );
        fixture.no_remote_effects();
    }

    #[test]
    fn explicit_description_overrides_generated_body() {
        let fixture = Fixture::new();
        let body_file = fixture.tools.path().join("reviewed.md");
        fs::write(&body_file, BODY).unwrap();
        fixture.response(
            "feat: #1470 generate PR metadata",
            "Unreviewed generated body.",
        );
        let output = fixture
            .command(&[
                "pr",
                "create",
                "--preview",
                "--description-file",
                body_file.to_str().unwrap(),
            ])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let data: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(data["description"], BODY);
        fixture.no_remote_effects();
    }

    #[test]
    fn branch_template_takes_precedence_over_global_default() {
        let fixture = Fixture::new();
        let template = "## Develop review\n<!-- Explain -->";
        fixture.template(
            ".azuredevops/pull_request_template/branches/develop.md",
            template,
        );
        fixture.template(
            ".vsts/pull_request_template/branches/develop.md",
            "## Lower priority\n<!-- Not selected -->",
        );
        fixture.response(
            "feat: #1470 generate PR metadata",
            "## Develop review\nDevelop changes.",
        );
        fixture
            .command(&["pr", "create", "--preview"])
            .assert()
            .success();
        assert_eq!(fixture.prompt_data()["template"], template);
        fixture.no_remote_effects();
    }

    #[test]
    fn unmatched_branch_template_does_not_constrain_stacked_prs() {
        let fixture = Fixture::new();
        fixture.template(
            ".azuredevops/pull_request_template/branches/develop.md",
            "## Develop only\n<!-- Not for stacked targets -->",
        );
        run_git(
            fixture.repo.path(),
            &["rm", ".azuredevops/pull_request_template.md"],
        );
        run_git(
            fixture.repo.path(),
            &["commit", "-m", "docs: #1470 remove global default"],
        );
        run_stack(fixture.repo.path(), &["create", "feat/1471-child"]).success();
        write_and_commit(
            fixture.repo.path(),
            "child.txt",
            "child change\n",
            "feat: #1471 child change",
        );
        fixture.response(
            "feat: #1471 add child behavior",
            "Add child behavior without the develop template.",
        );
        fixture
            .command(&["pr", "create", "--preview"])
            .assert()
            .success();
        let data = fixture.prompt_data();
        assert_eq!(data["target"], BRANCH);
        assert!(data["template"].is_null());
        fixture.no_remote_effects();
    }

    #[test]
    fn branch_templates_match_nested_targets_case_insensitively() {
        let fixture = Fixture::new();
        fixture.template(
            ".AZUREDEVOPS/PULL_REQUEST_TEMPLATE/BRANCHES/FEAT.md",
            "## Feature review\n<!-- Explain -->",
        );
        fixture.template(
            ".AZUREDEVOPS/PULL_REQUEST_TEMPLATE/BRANCHES/FEAT/1470-METADATA.txt",
            "## Nested review\n<!-- Explain -->",
        );
        run_stack(fixture.repo.path(), &["create", "feat/1471-child"]).success();
        write_and_commit(
            fixture.repo.path(),
            "child.txt",
            "child change\n",
            "feat: #1471 child change",
        );
        fixture.response(
            "feat: #1471 add child behavior",
            "## Nested review\nChild changes.",
        );
        fixture
            .command(&["pr", "create", "--preview"])
            .assert()
            .success();
        assert_eq!(
            fixture.prompt_data()["template"],
            "## Nested review\n<!-- Explain -->"
        );
        fixture.no_remote_effects();
    }

    #[test]
    fn generates_concise_metadata_without_a_template() {
        let fixture = Fixture::new();
        run_git(
            fixture.repo.path(),
            &["rm", ".azuredevops/pull_request_template.md"],
        );
        run_git(
            fixture.repo.path(),
            &["commit", "-m", "docs: #1470 remove template"],
        );
        fixture.response(
            "feat: #1470 generate PR metadata",
            "Generate useful PR metadata.",
        );
        fixture
            .command(&["pr", "create", "--preview"])
            .assert()
            .success()
            .stderr(predicates::str::contains("No PR template applies"));
        assert!(fixture.prompt_data()["template"].is_null());
        fixture.no_remote_effects();
    }

    #[test]
    fn rejects_unreadable_explicit_template_without_remote_effects() {
        let fixture = Fixture::new();
        fixture
            .command(&["pr", "create", "--template", "missing.md", "--yes"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("failed to read PR template"));
        assert!(!fixture.tools.path().join("copilot-args").exists());
        fixture.no_remote_effects();
    }

    #[test]
    fn refuses_large_context_and_sensitive_paths() {
        let fixture = Fixture::new();
        write_and_commit(
            fixture.repo.path(),
            "large.txt",
            &"x".repeat(100 * 1024),
            "feat: #1470 large change",
        );
        fixture
            .command(&["pr", "create", "--yes"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("generation limit"));
        assert!(!fixture.tools.path().join("copilot-args").exists());
        fixture.no_remote_effects();

        let fixture = Fixture::new();
        write_and_commit(
            fixture.repo.path(),
            ".env",
            "SECRET=not-real\n",
            "chore: #1470 config",
        );
        fixture
            .command(&["pr", "create", "--yes"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("potentially sensitive file"));
        assert!(!fixture.tools.path().join("copilot-args").exists());
        fixture.no_remote_effects();
    }
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

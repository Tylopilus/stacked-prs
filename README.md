# stacked-prs

Local-first CLI for managing stacked branches with squash-merge-friendly rebases.

## Current scope

- Tracks branch relationships locally in `.stacked-prs/state.json`
- Creates and tracks stacked branches
- Rebases a branch or the full stack when parent tips move
- Handles the common squash-merge flow by marking a parent as merged and rebasing children onto `develop`
- Optionally creates, reconciles, retargets, and lands Azure DevOps PRs with the `az` CLI
- Cleans up merged leaf branches safely

The core branch metadata is local-first and provider-independent. Azure DevOps support is optional and only used by the `pr`, `land`, and default PR reconciliation parts of `sync`.

## Important design note

Stack state lives inside the repo directory, but it is intentionally gitignored via `.gitignore`.

This keeps the metadata local to your clone while avoiding a major problem with tracked stack state: once different branches start carrying different versions of the metadata file, squash merges on parent branches can drop child stack metadata from `develop`.

Keeping the state repo-local but untracked preserves the local workflow while still making the data easy to inspect and back up.

## Commands

```text
stacked-prs init [--trunk develop] [--remote origin]
stacked-prs status [--json]
stacked-prs create <branch> [--parent <branch>]
stacked-prs track [branch] [--parent <branch>]
stacked-prs rebase <branch> [--onto <branch>] [--dry-run]
stacked-prs reparent <branch> --parent <branch> [--no-rebase] [--dry-run]
stacked-prs reparent --continue
stacked-prs reparent --abort
stacked-prs sync --all [--dry-run] [--push] [--no-pr]
stacked-prs mark-merged [branch]
stacked-prs pr create [branch] [--title <title>] [--draft]
stacked-prs pr sync
stacked-prs land [branch]
stacked-prs cleanup [--dry-run]
stacked-prs doctor
```

`init` is optional. The first `create` or `track` auto-initializes the repo with trunk `develop` and remote `origin` if no stack state exists yet.

## Typical flow

```bash
git checkout -b feature/a develop
stacked-prs track
stacked-prs create feature/b
stacked-prs status
stacked-prs sync --all
```

`track` defaults to the current branch and uses the configured trunk as parent when
`--parent` is omitted. If you run `create` from an untracked feature branch, the CLI
automatically tracks the current branch with trunk as its parent before creating the
new stacked child branch.

Use `init` only when you want to set non-default config explicitly:

```bash
stacked-prs init --trunk develop --remote origin
```

When `feature/a` is squash-merged into `develop`:

```bash
stacked-prs mark-merged feature/a
stacked-prs sync --all
stacked-prs cleanup --dry-run
stacked-prs cleanup
```

If the branch has a tracked Azure DevOps PR, `stacked-prs pr sync` or `stacked-prs sync --all` can detect the completed PR and mark the branch as merged automatically. `mark-merged` remains available as a manual override.

## Azure DevOps PR flow

Azure DevOps integration shells out to `az repos pr ...` from the repository root, so the Azure CLI must be installed, logged in, and able to infer the organization, project, and repository from the git remote.

```bash
stacked-prs pr create feature/a --title "Feature A"
stacked-prs pr create feature/b --draft
stacked-prs pr sync
stacked-prs land feature/a
stacked-prs sync --all --push
```

- `pr create` pushes the tracked branch, opens a PR targeting its effective parent, records the PR in local stack state, and refreshes the generated stack block in open PR descriptions
- `pr sync` reconciles tracked PRs with Azure DevOps, marks completed PRs as merged, drops abandoned PR references, retargets active PRs when their expected parent changes, and refreshes stack descriptions
- `land` sets Azure DevOps auto-complete on the bottom PR of the stack with squash merge and source-branch deletion enabled
- `sync --all` reconciles tracked PRs before planning rebases; use `--no-pr` to skip Azure DevOps calls
- `sync --all --push` force-pushes active tracked branches with lease so their PRs update after restacking

## Safety rules

- `rebase` and `sync` fail on a dirty worktree
- `sync --push` uses `--force-with-lease`
- `create` checks out the new branch immediately
- `cleanup` only deletes merged leaf branches and never deletes the current branch

## Development

```bash
cargo fmt
cargo test
```

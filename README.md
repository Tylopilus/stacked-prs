# stacked-prs

Local-first CLI for managing stacked branches with squash-merge-friendly rebases.

## Current scope

- Tracks branch relationships locally in `.stacked-prs/state.json`
- Creates and tracks stacked branches
- Rebases a branch or the full stack when parent tips move
- Handles the common squash-merge flow by marking a parent as merged and rebasing children onto `develop`
- Cleans up merged leaf branches safely

The tool is provider-agnostic today. It does not create or retarget pull requests yet.

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
stacked-prs sync --all [--dry-run]
stacked-prs mark-merged [branch]
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

## Safety rules

- `rebase` and `sync` fail on a dirty worktree
- `create` checks out the new branch immediately
- `cleanup` only deletes merged leaf branches and never deletes the current branch

## Development

```bash
cargo fmt
cargo test
```

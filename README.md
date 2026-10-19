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
stacked-prs sync --all [--from <branch>] [--dry-run] [--push] [--no-pr]
stacked-prs mark-merged [branch]
stacked-prs pr create [branch] [--title <title>] [--description-file <path>]
                      [--template <path>] [--no-ai] [--preview] [--yes] [--draft]
stacked-prs pr sync
stacked-prs land [branch]
stacked-prs cleanup [--dry-run]
stacked-prs doctor
stacked-prs completions <shell>
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
stacked-prs pr create feature/a --preview
stacked-prs pr create feature/a --title "feat: add feature A"
stacked-prs pr create feature/b --draft
stacked-prs pr sync
stacked-prs land feature/a
stacked-prs sync --all --push
```

- `pr create` generates and validates PR metadata, asks for confirmation, then pushes the tracked branch, opens a PR targeting its effective parent, records the PR in local stack state, and refreshes the generated stack block in open PR descriptions
- `pr sync` reconciles tracked PRs with Azure DevOps, marks completed PRs as merged, drops abandoned PR references, retargets active PRs when their expected parent changes, and refreshes stack descriptions
- `land` sets Azure DevOps auto-complete on the bottom PR of the stack with squash merge and source-branch deletion enabled
- `sync --all` reconciles tracked PRs before planning rebases; use `--no-pr` to skip Azure DevOps calls
- `sync --from <branch>` rebases only descendants of that branch and leaves the branch itself plus lower stack branches untouched
- `sync --all --push` force-pushes active tracked branches with lease so their PRs update after restacking

Rebases onto trunk use its fetched remote-tracking tip from the configured remote,
even when the local trunk branch has not been pulled. Active stacked parents use
their local tips so children follow parents rebased earlier in the same sync.

### AI-generated titles and descriptions

PR creation uses an installed, authenticated GitHub Copilot CLI (`copilot`) by
default. It sends the committed diff and commit subjects between the requested
branch and its effective PR target, plus any applicable description template. Uncommitted
worktree changes and lower stack changes are not included. Copilot runs with
tools, custom instructions, and built-in MCP servers disabled; your configured
model remains in effect.

Titles follow conventional commit syntax because they become squash commit
messages: `feat: #1470 generate PR metadata`. The primary work item comes from a
numeric branch prefix such as `feat/1470-description`, an explicit title, or a
single referenced commit work item, in that order. Ambiguous work items require
an explicit title or numbered branch. No PBI is invented when none is resolved.
The primary PBI and other commit work items are linked to the Azure PR.

Templates are read from the **committed source branch**, not the current worktree.
Discovery first checks target-branch-specific `.md` and `.txt` files under
`pull_request_template/branches/` in `.azuredevops/`, `.vsts/`, `docs/`, and the
repository root, in that order. The most specific matching branch prefix wins
within each location, up to ten branch levels; matching is case-insensitive.
For example, `.azuredevops/pull_request_template/branches/develop.md` applies
to PRs targeting `develop` or `develop/*`, not to PRs targeting `feat/*`.

If no branch template matches, discovery checks the global
`pull_request_template.md` and `.txt` in the same locations, followed by
`.github/PULL_REQUEST_TEMPLATE.md` and `.github/pull_request_template.md`.
Use `--template` for an explicit local file. With no applicable template,
Copilot still generates a commit-style title and a concise description without
imposing template sections; this is reported on stderr.

To scope a template to `develop`, keep only the branch-specific `develop.md`,
not a global default. Azure's web UI independently reads templates from the
repository's **default branch**, so merge the scoped template there to enable
automatic selection in the UI. It copies the template verbatim into the raw
description, including HTML guidance comments; authors must replace guidance
with real content. The Copilot flow is instructed to remove guidance comments.

The generated description must fill the template's `##` sections; sections whose
guidance says "Remove this section" are optional. Generation is instructed to
explain the problem, high-level solution, and rationale, without inventing test
results or visual evidence. Review the preview for accuracy. The stack overview
is appended separately and preserved by subsequent stack updates.

```bash
# Generate JSON without pushing, opening a PR, or changing stack state.
stacked-prs pr create feat/1470-metadata --preview

# Review the metadata interactively, then confirm creation.
stacked-prs pr create feat/1470-metadata --draft

# Use a specific template and skip the confirmation prompt.
stacked-prs pr create --template ./review-template.md --yes

# Supply reviewed metadata without calling Copilot.
stacked-prs pr create --no-ai --title "docs: #1470 document PR creation" \
  --description-file ./description.md --yes
```

`--title` and `--description-file` override their respective generated fields;
supplying both bypasses AI even without `--no-ai`. Manual descriptions are still
checked against a discovered or explicitly selected template, if present.
Non-interactive creation requires `--yes`; use `--preview` to review first.
Declining confirmation leaves the remote branch and PR untouched.

Unreadable explicit templates, invalid JSON, invalid titles, unfilled sections, Copilot
authentication/process failures, and generation timeouts fail **before any push
or PR creation**. Generation has a 120-second timeout, a 96 KiB prompt limit, and
a 256 KiB limit per output stream; oversized diffs are never silently truncated.
Changed `.env`, `.env.*`, `.pem`, and `.key` files are conservatively refused,
including example files; use manual metadata instead. This filename guard is
not a secret scanner: inspect committed changes before sending them to Copilot.

## Safety rules

- `rebase` and `sync` fail on a dirty worktree
- `sync --push` uses `--force-with-lease`
- `create` checks out the new branch immediately
- `cleanup` only deletes merged leaf branches and never deletes the current branch

Merged and archived branches are historical metadata: their local refs and parent
refs may be deleted without blocking `track`, `doctor`, or `sync`. Active branches
and their effective parents must still exist; children of merged branches use trunk.

## Shell Completions

Generate completions for your shell:

```bash
stacked-prs completions zsh > ~/.zfunc/_stacked-prs
```

Supported shells are `bash`, `elvish`, `fish`, `powershell`, and `zsh`.

## Development

```bash
cargo fmt
cargo test
```

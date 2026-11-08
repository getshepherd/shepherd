# Worktrees outside herdr

Rex, hc and tmux have no worktrees of their own: make one with git, where Shepherd puts its own, and run the
project's worktree hooks yourself, which Shepherd runs only for the worktrees it makes.

```bash
ROOT=$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")   # the main checkout
SLUG=$(printf %s "$BRANCH" | tr -cs 'A-Za-z0-9' - | tr A-Z a-z | sed 's/^-*//; s/-*$//')
WT=~/.herdr/worktrees/$(basename "$ROOT")/$SLUG
git -C "$ROOT" worktree add -b "$BRANCH" "$WT" "$BASE"
```

`worktrees.directory` in `~/.config/herdr/config.toml`, when set, replaces `~/.herdr/worktrees`.

## Hooks

The commands to run in a new worktree are the project's `created =` lines in
`~/.config/herdr/plugins/config/shepherd.worktree-hooks/hooks.conf` (Settings → Worktrees in Shepherd writes them).
A project is a `[name]` or `[session/name]` section whose `dir =` is `$ROOT`, `~` meaning the home folder. Run each
line in order, from `$WT`, with `sh -c` and `ROOT_PATH=$ROOT WORKTREE_PATH=$WT PROJECT=<name> EVENT=created`; the
first that fails stops the rest. Before removing a worktree, run its `removed =` lines the same way with
`EVENT=removed`.

Remove it with `git -C "$ROOT" worktree remove "$WT"`, after the checks in Clean up above.

# Agents on herdr

herdr has agents of its own: `herdr agent start` runs one in a pane, `agent prompt` briefs it, `agent wait`,
`agent get` and `agent read` follow it. Every other pane, tab and workspace command: `herdr --skill` prints herdr's
own guide.

## Where

- A tab beside you:
  ```bash
  WS=$(herdr pane current | jq -r .result.pane.workspace_id)
  PANE=$(herdr tab create --workspace "$WS" --label "$NAME" --no-focus | jq -r .result.root_pane.pane_id)
  ```
- Its own worktree workspace, from the repository you are in:
  ```bash
  R=$(herdr worktree create --branch "$BRANCH" --base "$BASE" --label "$LABEL" --cwd "$PWD" --no-focus)
  PANE=$(jq -r .result.root_pane.pane_id <<<"$R")
  WS=$(jq -r .result.workspace.workspace_id <<<"$R")
  ```
  Without `--cwd` the worktree is made in the active workspace's repository, which may be another project. Keep
  `$BRANCH` and `$WS`. Pane ids compact as panes close: re-read the pane from `herdr agent list` before each later use.

Worktrees come and go only through `herdr worktree create` and `remove`: they fire the events Shepherd's worktree
hooks run on (`.env`, dependencies, local settings); `git worktree add` and `remove` fire none.

## Start and brief

```bash
herdr agent start "$NAME" --kind claude --pane "$PANE" --timeout 60000
herdr agent prompt "$PANE" "$(cat "$BRIEF")"
```

## Follow

```bash
herdr agent wait "$PANE" --until idle --until done --timeout 3600000
herdr agent read "$PANE" --lines 100
herdr agent prompt "$PANE" "…" --wait --until idle --timeout 1800000   # a follow-up
```

`agent wait` takes one pane: run one per agent in the background and wait for them all.

## Clean up

`herdr tab close <tab>` for a tab; `herdr worktree remove --workspace "$WS"` for a worktree, after the checks in Clean up
above.

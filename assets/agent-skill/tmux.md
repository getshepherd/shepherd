# Agents on tmux

In Shepherd a tmux session is a workspace, a window is a tab and a pane is a pane. You are `$TMUX_PANE`.

## Where and start

tmux carries no agent state, so the agent reports back through a file: end every brief with "When you are done,
overwrite `$REPORT` with your report." Keep `$REPORT` outside its checkout.

```bash
REPORT=$(mktemp "${TMPDIR:-/tmp}/$NAME.XXXXXX")
```

- A tab beside you, in your session:
  ```bash
  SESSION=$(tmux display -p -t "$TMUX_PANE" '#{session_id}')
  P=$(tmux new-window -d -P -F '#{pane_id}' -t "$SESSION:" -n "$NAME" -c "$DIR" claude "$(cat "$BRIEF")")
  ```
- Its own worktree: make it as "Worktrees outside herdr" below says, then give it a session of its own:
  ```bash
  P=$(tmux new-session -d -P -F '#{pane_id}' -s "$NAME" -c "$WT" claude "$(cat "$BRIEF")")
  ```

Keep `$P`. The trust dialog: `tmux send-keys -t "$P" Down Enter`.

## Follow

```bash
until [ -s "$REPORT" ] || ! tmux list-panes -a -F '#{pane_id}' | grep -qx "$P"; do sleep 10; done   # done, or its pane is gone
cat "$REPORT"
tmux capture-pane -p -J -t "$P" -S -100   # its screen and the last 100 lines
```

Run the wait in the background for a long turn. A follow-up empties the report first and goes in as one bracketed
paste:

```bash
: > "$REPORT"
printf %s "$TEXT" | tmux load-buffer -b brief - && tmux paste-buffer -p -d -b brief -t "$P" && tmux send-keys -t "$P" Enter
```

## Clean up

`tmux kill-window -t "$P"` for a tab; `tmux kill-session -t "$NAME"` for a worktree's session, then remove the
worktree as "Worktrees outside herdr" below says. Remove `$REPORT`.

# Agents on Rex

In Shepherd a Rex session is a workspace, a window is a tab and a block is a pane. You are `$REX_SESSION` and
`$REX_BLOCK`. Every other command: `rex __internal skill` prints Rex's own guide.

## Where and start

- A tab beside you:
  ```bash
  S=$REX_SESSION
  B=$(rex window new -s "$S" "$NAME" -c "$DIR" --focus=false --json -- claude "$(cat "$BRIEF")" | jq -r '.block_ids[0]')
  ```
- Its own worktree: make it as "Worktrees outside herdr" below says, then give it a session of its own:
  ```bash
  R=$(rex new "$NAME" -c "$WT" --json -- claude "$(cat "$BRIEF")")
  S=$(jq -r .session_id <<<"$R"); B=$(jq -r '.initial_windows[0].block_ids[0]' <<<"$R")
  ```

Keep `$S` and `$B`. The trust dialog: `rex send-key -s "$S" -b "$B" down && rex send-key -s "$S" -b "$B" enter`.

## Follow

Claude Code reports its state to Rex: `program_status` holds `working`, then `done` (or `idle`), `blocked` on a
question or a permission, or `error`, with the time it changed. A turn has ended once the state is not `working` and
its time differs from the one read before the prompt:

```bash
turn_end() {   # $1: updated_at from before the prompt, empty for the first turn; prints the state it ended in
  while :; do
    read -r s u <<<"$(rex block call -s "$S" -b "$B" program_status | jq -r '.records[0] | "\(.state) \(.updated_at)"')"
    case $s in done|idle|blocked|error) [ "$u" != "$1" ] && { echo "$s"; return; } ;; esac
    sleep 5
  done
}
turn_end ""
rex capture -s "$S" -b "$B" --trim | tail -n 100
```

No record at all (`null`) is an agent still starting, on the trust dialog, or one that reports no state: read its
screen. Run `turn_end` in the background for a long turn.

A follow-up:

```bash
before=$(rex block call -s "$S" -b "$B" program_status | jq -r '.records[0].updated_at')
rex send -s "$S" -b "$B" $'\e[200~'"$TEXT"$'\e[201~' && rex send-key -s "$S" -b "$B" enter
turn_end "$before"
```

## Clean up

`rex window close -s "$S" <window>` for a tab (`rex block inspect -s "$S" "$B" --json` has its `.block.window_id`);
`rex kill "$S"` for a worktree's session, then remove the worktree as "Worktrees outside herdr" below says.

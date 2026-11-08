# Agents on hc

`hc --skill` prints hc's own manual: read it first. It covers starting an agent with its task
(`hc run --bg --name "$NAME" --cwd "$DIR" claude "$(cat "$BRIEF")"`), its templates (`hc templates`), waiting
(`hc wait`, `hc wait --children all`), reading (`hc read`), follow-ups (`hc send`) and ending it (`hc kill`, `hc rm`).
`$HC_TERMINAL_ID` is your address, and the terminals you start are your children.

hc keeps no layout: Shepherd keeps it, and shows a terminal it did not open itself under **Other terminals** in its
sidebar. So both kinds of agent, a read-only one and a worktree one, are terminals you start with `hc run`; for a
worktree, make it first as "Worktrees outside herdr" below says and start the agent with `--cwd "$WT"`.

The trust dialog: `hc keys "$NAME" down enter`.

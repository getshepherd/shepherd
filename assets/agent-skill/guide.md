# Agents in Shepherd

Shepherd is a window onto one terminal multiplexer, its backend. A subagent here is an interactive agent in a
terminal of that backend: Shepherd's sidebar shows it with its status, and the user can read it, answer it and take
it over. You start it, brief it, follow it and clean up after it with the backend's own CLI: the steps come first,
then the commands of the backend this terminal runs on.

## Where the agent lives

- **A new tab beside you**, one per agent you start on your own that only reads and whose result comes back to you:
  a review, research, checking a hypothesis, anything short the user will not follow on its own.
- **Its own worktree, as its own workspace**: the user asked for a new agent (start it there right away, no
  questions, branched from the current branch), or it edits code: parallel subtasks you merge into one branch, or
  work that stands on its own (its own branch and PR, outlives this conversation, the user follows it directly).

## Brief and start

The agent knows only its brief: the goal, what is already ruled out, the files to read, the scope, and what to report
back. Paths are relative to its own checkout. In a fresh worktree add: "dependencies may still be installing; on a
missing-dependency failure, wait and retry."

Give the brief as the agent's first prompt at launch (`claude "<brief>"`): text typed into an agent that is still
starting can be lost. Several agents: write every brief, start them all, then follow them.

A folder the agent has not opened before (every fresh worktree) starts on Claude Code's trust dialog, and the agent
reports nothing until it is answered. Read the screen of an agent that has said nothing for a minute; on that dialog,
press down then enter.

## Follow

Wait for the turn to end, then read the agent's screen before acting on what it did. A wait that times out means it
is still working. A follow-up goes in as one bracketed paste and then enter, so a multi-line message arrives whole.
An agent that is blocked on a question or a permission: answer it if it is yours to answer, else tell the user.

## Clean up

- Close the tabs you opened once their result is in.
- A worktree comes down when `git -C <path> status --porcelain` is empty and its branch is merged:
  `git branch --merged <base>`, or `gh pr view <branch>` says `MERGED` (a squash merge hides from `--merged`). Then
  remove it the backend's way and `git branch -d <branch>`; `-D` for a PR confirmed merged.
- Uncommitted changes or an unmerged branch: show them and ask. Force only with the user's consent.
- Remove what you created for the current task. Others only when the user names them; to tidy up, list the merged,
  empty and prunable candidates and ask first.

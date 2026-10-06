---
name: linear-issue
description: Work on a Linear issue of the BongBong team end to end - read it, claim it, branch, implement, verify, and (when asked) open the PR that closes it. Use when the user names an issue (BB-21, "bb 21", a linear.app/bongbong/issue/... link) or says "work on", "pick up", "fix", "do" an issue, or "ship BB-N" - and before any other change to the repo, since every change needs an issue.
---

# Working a Linear issue

The team is **BongBong**, key `BB`, workspace `linear.app/bongbong`. The
argument is the issue: `BB-21`, `21`, or its URL. No issue given and the
request changes something: find the issue it belongs to
(`list_issues {team: "BongBong", query}`), or file one first with the
`linear-file` skill.

## The rules (MUST - CLAUDE.md "Development rules")

1. No work that Linear does not trace: every change belongs to an issue.
2. Starting work: the issue is **In Progress**, assigned to `me`, before the
   first edit, and carries a comment naming this session so Oto can resume
   it (see "The session comment").
3. Needing a human - a question, a clarification, an approval, a choice,
   finished work waiting for review: the issue goes to **AI Blocked** with a
   comment saying exactly what is needed, ending with the resume line, *then*
   ask. When the answer comes,
   back to **In Progress** before continuing.

A rule that cannot be kept (Linear unreachable, a one-line fix Oto wants done
now): ask Oto to bypass it and wait; never skip it on your own.

## The session comment

Once per session that works on the issue - when it is claimed, or when a
later session picks it up - `save_comment`:

```
Claude session `<id>` working on this.
Resume: `cd <dir> && claude --resume <id>`
Branch: `<branch>`
```

- `<id>` is `$CLAUDE_CODE_SESSION_ID` (`echo $CLAUDE_CODE_SESSION_ID`); `<dir>`
  is the directory the session started in - sessions are kept per directory,
  so a session that entered a worktree is still resumed from where it
  started, and one started in a worktree from that worktree.
- On claude.ai/code (no `CLAUDE_CODE_SESSION_ID`), the session's
  `https://claude.ai/code/session_...` URL instead, if known; if neither is
  known, say so in the comment rather than leave it out.
- An AI Blocked comment ends with the same `Resume:` line.

## 1. Read the issue whole

- `get_issue`, `list_comments` on it, and `get_issue` for its parent and any
  sub-issues (`list_issues {parentId}`). Images in the description:
  `extract_images`. Comments often carry Oto's later feedback - read them
  before taking the description as the spec.
- A vague issue (one line, "Revisit X", "too intense") is a play-test note:
  read the docs the module map names for that area, reproduce it with the dev
  server or the probe, and say in the report what "done" was taken to mean.
  If two readings would build different things, that is a question: rule 3.

## 2. Claim it

- `save_issue`: assignee `me`, state **In Progress** (rule 2), then the
  session comment.
- Labels only if it has none: one type (`Feature`, `Bug`, `Improvement`) and
  the areas that fit (`gameplay`, `Maps`, `HUD`, `Design`, `server`,
  `security`).

## 3. Branch

- The branch is the issue's `gitBranchName` exactly
  (`otobrglez/bb-21-granade-launcher`) - Linear links the PR by it.
  `git fetch origin && git switch -c <gitBranchName> origin/master`, or
  `git switch <gitBranchName>` if it exists (`git branch -a --list '*bb-<n>-*'`).
- Oto often runs several sessions on this tree. If `git status` is not clean
  or `ListAgents` shows a peer, use a worktree (`EnterWorktree`) on that
  branch instead of switching the shared tree.
- Two issues that are one change (BB-25 + BB-26) share the first one's branch;
  both are In Progress.

## 4. Do the work

Follow CLAUDE.md. A comment that promises a mechanic gets a `mechanics_tests`
case; a tuning change is a `tunables!` row; a rendering change is checked on a
screenshot through the dev server. Work found that is not this issue (a
pre-existing failure, a follow-up): file it with `linear-file`, don't fold it in.

## 5. Verify and hand over

- Run what the change touches (at least `cargo test --lib --no-default-features
  --features dev-tools`; `just probe-fixtures` for AI, map or tuning changes;
  `cargo test -p bongbong-server` for `net/` or `server/`).
- Unless "ship" was asked for, the work now waits for Oto: set **AI Blocked**,
  `save_comment` what was done, how it was verified (failures included) and
  what is needed ("review the change, then say ship"), and report the same.

## 6. Ship (only when asked)

- Back to **In Progress** if it was AI Blocked.
- Commit subject: what changed, in plain words, then the id:
  `Keep the frog's training lines up longer (BB-23)`. One commit per issue
  when a branch carries two.
- Push, then `gh pr create --base master` with:
  - **Title**: `<what changed> (BB-N)`, the commit subject's shape. The squash
    merge appends `(#PR)`, which `tools/release/gather.sh` reads.
  - **Body**, first line: `Fixes BB-N` - one line per issue the PR finishes
    (Linear moves each to Done on merge); `Part of BB-N` for one it only
    advances. Then what a player or tester sees, how it was verified, and
    anything re-baselined on purpose - the release notes are written from PR
    bodies, so lead with the player's view.
- The GitHub integration moves the issue to In Review when the PR opens and to
  Done when it merges; don't set those by hand.
- `save_comment` on the issue: the PR link, plus anything its reader should
  know that the PR doesn't say (a follow-up filed, a reading of a vague issue).

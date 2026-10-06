---
name: linear-file
description: File a bug, follow-up, idea or play-test note as a Linear issue in the BongBong team, labelled and worded so it can be picked up later. Use when the user says "file", "log", "track", "make an issue/ticket for", "put it in Linear", or when a session finds work outside its current issue (a pre-existing failure, a follow-up, a security alert).
---

# Filing a Linear issue

Team **BongBong** (key `BB`). One issue per thing a person would pick up and
finish in one PR; a bigger idea is a parent with sub-issues (`parentId`), as
BB-5 "The mushroom hunt" holds BB-6 and BB-7.

## Before creating

`list_issues {team: "BongBong", query: "<key words>"}`, open issues and the
recently done ones. If one already covers it, add a comment there instead and
say so.

## The issue

- **Title**: the problem or the outcome in plain words, as a player or Oto
  would say it ("Frog's speech bubbles vanish before they can be read"), not
  the module name. No `BB-` prefix, no trailing period.
- **Description** (Markdown, real newlines):
  - What happens and what should happen instead; for a bug, the replay recipe
    if there is one - map, `--seed`, mission, the dev-server or probe steps
    (`restart {seed, map}`, `step {frames}`), or a probe `ANOMALY` line.
  - Where it lives: the files or docs the CLAUDE.md module map points to, so
    the session that picks it up starts in the right place.
  - Where it came from: the PR or issue that found it (`Found while working
    on BB-21`, `#96`), a Dependabot alert link, or "play-test".
  - A screenshot from the dev server if the bug is visual: upload with
    `prepare_attachment_upload` / `create_attachment_from_upload`.
- **Labels**: exactly one type - `Bug` (something a player can run into that is
  wrong), `Feature` (something new), `Improvement` (something existing made
  better) - plus every area that fits: `gameplay`, `Maps`, `HUD`, `Design`
  (art, look, feel), `server` (rooms, co-op, deploy), `security`. Ask before
  inventing a new label.
- **State**: `Backlog` unless Oto says it is next (`Todo`) or it is filed for
  the work about to start - every change needs an issue first (CLAUDE.md
  "Development rules") - then `In Progress`, assigned to `me`, with the
  session comment, and the work goes on under the `linear-issue` skill.
- **Priority**: leave it unset unless it is a security issue, a crash or a
  broken release path (then `High`, or `Urgent` for production down), or Oto
  gives one.
- **Parent**: set `parentId` when it is a step of an existing bigger issue.

## After

Reply with the id and its URL, one line. Don't start the work unless asked
or it was filed for the work in hand; the work is the `linear-issue` skill.

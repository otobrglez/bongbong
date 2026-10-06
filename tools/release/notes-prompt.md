# Writing a release's notes

You are adding one release's entry to `CHANGELOG.md` at the repository root.
The caller tells you the version, the date and the path of a context file made
by `tools/release/gather.sh`: every pull request merged into master since the
previous release (title, labels, body), the stacked PRs folded into them, and
the commits made straight onto master. Read `CHANGELOG.md` first and match the
entries already there.

## Who reads it

The entry is published as the GitHub release's notes, above the download
table, and its heading becomes the release's title (cargo-dist reads it). The
readers are **players and testers**, not the people who wrote the code. They
want to know what is new to play, what feels different and what was fixed.

## What to write

Insert the entry directly under the file's introduction, above the newest
entry already there. Touch nothing else in the file.

```
## <version> - <YYYY-MM-DD>

One or two sentences on what this release is about, in plain words.

### New
- ...

### Changed
- ...

### Fixed
- ...

### Online
- ...

### Platforms
- ...

### Behind the scenes
- ...
```

- The heading is exactly `## <version> - <date>`, with the version as given
  (no `v`), or the release will go out without its notes.
- Leave out any section that would be empty. Keep the order above.
- **New**: things a player can now do or meet (weapons, levels, maps,
  mechanics, builder tools, languages). **Changed**: what plays, looks or
  feels different. **Fixed**: bugs a player could have run into. **Online**:
  co-op rooms and networked play. **Platforms**: web, iOS, Android, desktop
  builds and how to get them. **Behind the scenes**: CI, tooling, servers,
  monitoring, refactors - at most three bullets, one line each, or leave it
  out.
- One line per bullet, two at most. Start with what the player sees, not with
  the module or file that changed. Name things the way the game's screens do
  (BUILD, PLAY HERE, Boot Camp, LEVEL 0).
- End a bullet with the pull request it came from, as `(#95)`. Several PRs
  behind one change: `(#71, #73)`. A commit made straight onto master gets no
  reference.
- Fold a fix to something that is new in the same release into that thing's
  bullet; a release never fixes what it introduced.
- Leave out Linear ids (`BB-12`), branch names, tuning-knob names, file paths,
  test names and internal design-doc references.
- Aim for 6 to 20 bullets in all. Group many small related PRs into one bullet
  rather than listing each.
- Write only what the context supports, and never make up an example, a name
  or a number the context does not give. If a PR's purpose is unclear, describe
  it from its title alone, briefly, or leave it out if it is invisible to a
  player and not worth a "Behind the scenes" line.
- Plain Markdown: no emoji, no tables, no images, no HTML.

Write the entry with the file tools, then stop. Do not commit, push or touch
any other file.

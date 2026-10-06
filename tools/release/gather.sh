#!/usr/bin/env bash
# What a release is made of, as one Markdown document for whoever writes its
# notes (tools/release/notes-prompt.md). Usage:
#   tools/release/gather.sh [SINCE_TAG] [REF]
# SINCE_TAG defaults to the newest v* tag reachable from REF (default HEAD).
# Every pull request whose merge or squash commit is in SINCE_TAG..REF is
# listed with its title, labels and body - the ones merged into master in
# full, the stacked ones (merged into another PR's branch) by title only -
# then the commits made straight onto master. Needs `gh` logged in (GH_TOKEN
# in CI).
set -euo pipefail
REF="${2:-HEAD}"
SINCE="${1:-$(git describe --tags --abbrev=0 --match 'v*' "$REF")}"
BODY_MAX=8000

echo "# Changes from $SINCE to $REF"
echo
echo "Previous release: $SINCE ($(git log -1 --format=%cs "$SINCE"))."

# A merge commit says "Merge pull request #N"; a squash ends its subject "(#N)".
nums="$(git log --format=%s "$SINCE..$REF" \
    | sed -nE -e 's/^Merge pull request #([0-9]+).*/\1/p' -e 's/.*\(#([0-9]+)\)$/\1/p' \
    | sort -un)"

stacked=""
echo
echo "## Pull requests merged into master"
for num in $nums; do
    pr="$(gh pr view "$num" --json number,title,labels,body,baseRefName)"
    base="$(jq -r .baseRefName <<<"$pr")"
    if [[ "$base" != "master" ]]; then
        stacked+="- #$num $(jq -r .title <<<"$pr") (into $base)"$'\n'
        continue
    fi
    jq -r '"\n### #\(.number) \(.title)\n\nLabels: \([.labels[].name] | join(", ") | if . == "" then "none" else . end)\n"' <<<"$pr"
    # The body quoted, so its own headings stay inside it.
    jq -r '.body // ""' <<<"$pr" | head -c "$BODY_MAX" | sed 's/^/> /'
    echo
done

echo
echo "## Stacked pull requests"
echo
echo "Merged into another PR's branch: they reached master inside one of the PRs above. Use them for detail, never as entries of their own."
echo
printf '%s' "${stacked:-- none}"$'\n'

echo
echo "## Commits made straight onto master"
echo
git log --first-parent --no-merges --format=$'%h\t%s' "$SINCE..$REF" \
    | grep -vE '\(#[0-9]+\)$' \
    | while IFS=$'\t' read -r sha subject; do
        echo "- $sha $subject"
        git log -1 --format=%b "$sha" | head -c 1000 | sed 's/^/  /'
    done

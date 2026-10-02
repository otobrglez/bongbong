#!/usr/bin/env bash
# Checks tools/ios/version.sh, the version and build number every iOS
# bundle carries (CI runs it; no Mac needed).
set -uo pipefail
cd "$(dirname "$0")/../.."
unset BONGBONG_IOS_PR BONGBONG_IOS_BUILD
VERSION_SH=tools/ios/version.sh
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
FAILED=0
ok() { echo "ok   $*"; }
bad() { echo "FAIL $*"; FAILED=1; }

cargo_toml() { # a Cargo.toml carrying version $1
    printf '[package]\nname = "bongbong"\nversion = "%s"\nedition = "2024"\n\n[dependencies]\nfoo = { version = "9.9.9" }\n' "$1" > "$TMP/Cargo.toml"
    echo "$TMP/Cargo.toml"
}

# expect NAME WANT_VERSION BUILD_REGEX [ENV=VALUE...] -- [ARGS...]
expect() {
    local name="$1" want_ver="$2" build_re="$3"; shift 3
    local envs=()
    while [[ $# -gt 0 && "$1" != -- ]]; do envs+=("$1"); shift; done
    [[ $# -gt 0 ]] && shift
    local out
    if ! out="$(env ${envs[@]+"${envs[@]}"} "$VERSION_SH" "$@" 2>&1)"; then
        bad "$name: failed: $out"; return
    fi
    local ver build
    read -r ver build <<<"$out"
    if [[ "$ver" == "$want_ver" && "$build" =~ $build_re ]]; then
        ok "$name: $ver ($build)"
    else
        bad "$name: got '$out', want version $want_ver and a build matching $build_re"
    fi
}

# refuse NAME [ENV=VALUE...] -- [ARGS...]
refuse() {
    local name="$1"; shift
    local envs=()
    while [[ $# -gt 0 && "$1" != -- ]]; do envs+=("$1"); shift; done
    [[ $# -gt 0 ]] && shift
    local out
    if out="$(env ${envs[@]+"${envs[@]}"} "$VERSION_SH" "$@" 2>&1)"; then
        bad "$name: accepted, printed '$out'"
    else
        ok "$name: refused ($out)"
    fi
}

STAMP='^[0-9]{8}\.[0-9]{4}'
C="$(cargo_toml 0.2.5)"

# A release is Cargo.toml's version at the UTC minute.
expect "release" 0.2.5 "$STAMP\$" -- "$C"
expect "the repo's own Cargo.toml" "$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)" "$STAMP\$" --
expect "release, build pinned" 0.2.5 '^42$' BONGBONG_IOS_BUILD=42 -- "$C"

# A pull request gets a train of its own, and its number ends the build.
expect "PR 68" 0.2.10068 "$STAMP\.68\$" BONGBONG_IOS_PR=68 -- "$C"
expect "PR 1" 0.2.10001 "$STAMP\.1\$" BONGBONG_IOS_PR=1 -- "$C"
expect "PR 12345" 0.2.22345 "$STAMP\.12345\$" BONGBONG_IOS_PR=12345 -- "$C"
expect "PR 68, build pinned" 0.2.10068 '^7\.8\.68$' BONGBONG_IOS_PR=68 BONGBONG_IOS_BUILD=7.8.68 -- "$C"
expect "PR 68 on 1.0.0" 1.0.10068 "$STAMP\.68\$" BONGBONG_IOS_PR=68 -- "$(cargo_toml 1.0.0)"
expect "PR 68 on 0.2.9999" 0.2.10068 "$STAMP\.68\$" BONGBONG_IOS_PR=68 -- "$(cargo_toml 0.2.9999)"

# A pre-release keeps its own version (App Store Connect's to judge, and a
# simulator build's to carry); its PRs take its MAJOR.MINOR.
expect "a pre-release" 0.3.0-beta.1 "$STAMP\$" -- "$(cargo_toml 0.3.0-beta.1)"
expect "PR 68 on a pre-release" 0.3.10068 "$STAMP\.68\$" BONGBONG_IOS_PR=68 -- "$(cargo_toml 0.3.0-beta.1)"
C="$(cargo_toml 0.2.5)"

# What App Store Connect would refuse, refused here first.
refuse "PR abc" BONGBONG_IOS_PR=abc -- "$C"
refuse "PR 0" BONGBONG_IOS_PR=0 -- "$C"
refuse "PR 068 (bash would read it as octal)" BONGBONG_IOS_PR=068 -- "$C"
refuse "PR -3" BONGBONG_IOS_PR=-3 -- "$C"
refuse "PR #68" "BONGBONG_IOS_PR=#68" -- "$C"
refuse "PR 68 on a two-number version" BONGBONG_IOS_PR=68 -- "$(cargo_toml 0.3)"
refuse "a Cargo.toml with no version" -- "$(printf '[package]\nname = "x"\n' > "$TMP/Cargo.toml"; echo "$TMP/Cargo.toml")"
C="$(cargo_toml 0.2.5)"
refuse "a four-number build" BONGBONG_IOS_BUILD=1.2.3.4 -- "$C"
refuse "a build with letters" BONGBONG_IOS_BUILD=20261002.0441-pr68 -- "$C"
refuse "no Cargo.toml" -- "$TMP/missing.toml"

# Every PR's version is its own and above the release it is built on, and
# no release whose patch stays below the offset can take it.
newer() { # is version $1 above $2, number by number (App Store Connect's order)
    local IFS=.
    local -a a=($1) b=($2)
    local i
    for i in 0 1 2; do
        (( ${a[i]:-0} > ${b[i]:-0} )) && return 0
        (( ${a[i]:-0} < ${b[i]:-0} )) && return 1
    done
    return 1
}
seen=" "
clash=""
for pr in 1 2 9 10 68 99 100 999 1000 9999 10000 12345; do
    read -r ver build < <(BONGBONG_IOS_PR=$pr "$VERSION_SH" "$C")
    [[ "$seen" == *" $ver "* ]] && clash="$clash $pr"
    seen="$seen$ver "
    newer "$ver" 0.2.5 || clash="$clash $pr(not-above-0.2.5)"
    newer 0.2.9999 "$ver" && clash="$clash $pr(below-0.2.9999)"
    # The `expire` job's test: three numbers, the last one the PR's.
    IFS=. read -r -a parts <<<"$build"
    [[ ${#parts[@]} -eq 3 && "${parts[2]}" == "$pr" ]] || clash="$clash $pr(build-$build)"
done
[[ -z "$clash" ]] && ok "PR versions are distinct, above their release, and their builds name the PR" \
    || bad "PR versions:$clash"
read -r ver build < <("$VERSION_SH" "$C")
IFS=. read -r -a parts <<<"$build"
[[ ${#parts[@]} -eq 2 ]] && ok "a release's build has two numbers, so \`expire\` never takes one" \
    || bad "a release's build $build has ${#parts[@]} numbers"

[[ $FAILED -eq 0 ]] && echo "all passed" || { echo "some checks failed"; exit 1; }

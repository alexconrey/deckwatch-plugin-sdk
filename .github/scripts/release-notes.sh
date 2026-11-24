#!/usr/bin/env bash
# Usage: release-notes.sh <tag>   (e.g. v0.10.0)
#
# Checks that the tag can be released and prints its release notes:
#   - the tag is v<version> and <version> equals Cargo.toml's package version;
#   - CHANGELOG.md has a "## [<version>] - YYYY-MM-DD" section (a dated one,
#     so "Unreleased" can't be released) with some content.
# Prints the section body, then a pinning hint. Exits non-zero, with a GitHub
# Actions error annotation, if any check fails.
set -euo pipefail

tag="${1:?usage: release-notes.sh <tag>}"
fail() { echo "::error file=$1::$2" >&2; exit 1; }

case "$tag" in
  v[0-9]*) version="${tag#v}" ;;
  *) fail Cargo.toml "tag '$tag' is not of the form vX.Y.Z" ;;
esac

cargo_version=$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' Cargo.toml | head -n1)
[ "$version" = "$cargo_version" ] ||
  fail Cargo.toml "tag $tag does not match Cargo.toml version $cargo_version"

heading=$(grep -F -- "## [$version]" CHANGELOG.md | head -n1 || true)
[ -n "$heading" ] || fail CHANGELOG.md "no '## [$version]' section"
printf '%s\n' "$heading" | grep -Eq '^## \[[^]]+\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$' ||
  fail CHANGELOG.md "'$heading' needs a release date: '## [$version] - YYYY-MM-DD'"

# The section body: after the heading, up to the next "## " heading or the
# link references at the end, without leading or trailing blank lines.
notes=$(awk -v h="$heading" '
  $0 == h { found = 1; next }
  found && (/^## / || /^\[[^]]+\]: /) { exit }
  found { print }
' CHANGELOG.md | sed -e '/./,$!d' | sed -e ':a' -e '/^\n*$/{$d;N;ba' -e '}')
[ -n "$notes" ] || fail CHANGELOG.md "the '## [$version]' section is empty"

repo="${GITHUB_REPOSITORY:-alexconrey/deckwatch-plugin-sdk}"
printf '%s\n\n---\n\nPin this release in a plugin:\n\n```toml\ndeckwatch-plugin-sdk = { git = "https://github.com/%s", tag = "%s" }\n```\n' \
  "$notes" "$repo" "$tag"

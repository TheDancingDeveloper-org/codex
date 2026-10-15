#!/usr/bin/env bash
# Drift check for the fork's upstream sync cadence (WI-1125).
#
# Read-only: it fetches tags and fails when the fork has fallen behind, but it
# never rebases, pushes or edits a workflow. The rebase itself stays the
# runbook at docs/fork-upstream-sync.md, because replaying the carried series
# needs a person when a patch no longer applies.
#
# Two checks:
#   1. The upstream stable tag recorded in FORK.md and the upstream_version
#      stamp in .github/workflows/fork-release.yml match the newest rust-vX.Y.Z
#      tag on the upstream remote. alpha and dev tags are skipped.
#   2. The carried-patch table in FORK.md has at most CARRIED_PATCH_CAP rows
#      (about 10, the target in FORK.md).
set -euo pipefail

cap="${CARRIED_PATCH_CAP:-10}"
remote="${UPSTREAM_REMOTE:-upstream}"
root="$(git rev-parse --show-toplevel)"

git -C "$root" fetch --tags "$remote"

# Newest stable tag by version, not by tag date. `sort -V` orders the numeric
# suffix; a tag with a pre-release suffix (alpha, dev) is excluded first.
newest="$(
  git -C "$root" tag -l 'rust-v*' |
    grep -E '^rust-v[0-9]+(\.[0-9]+){2}$' |
    sort -V |
    tail -n1
)"
if [[ -z "$newest" ]]; then
  echo "no stable rust-vX.Y.Z tag found on ${remote}" >&2
  exit 1
fi
newest_version="${newest#rust-v}"

recorded_tag="$(
  grep -oE 'rust-v[0-9]+(\.[0-9]+){2}' "$root/FORK.md" |
    head -n1
)"
stamped_version="$(
  sed -n 's/^[[:space:]]*upstream_version="\([0-9][0-9.]*\)".*/\1/p' \
    "$root/.github/workflows/fork-release.yml" |
    head -n1
)"

failed=0
if [[ "$recorded_tag" != "$newest" ]]; then
  echo "FORK.md records base ${recorded_tag:-<none>}; newest stable tag is ${newest}" >&2
  failed=1
fi
if [[ "$stamped_version" != "$newest_version" ]]; then
  echo "fork-release.yml stamps upstream_version=${stamped_version:-<none>}; newest is ${newest_version}" >&2
  failed=1
fi

# A carried-patch row is a table row under "## Carried patches" whose first
# cell is not the header and not the separator.
count="$(
  awk '
    /^## Carried patches/ { in_table = 1; next }
    in_table && /^## / { exit }
    in_table && /^\| — \|/ { n++ }
    END { print n + 0 }
  ' "$root/FORK.md"
)"
if (( count > cap )); then
  echo "carried-patch count is ${count}, over the cap of ${cap}" >&2
  failed=1
fi

if (( failed )); then
  echo "drift: rebase onto ${newest} per docs/fork-upstream-sync.md" >&2
  exit 1
fi

echo "in sync: ${newest}, ${count} carried patch(es), cap ${cap}"

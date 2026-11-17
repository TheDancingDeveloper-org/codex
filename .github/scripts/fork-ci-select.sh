#!/usr/bin/env bash
# fork-ci's package selection: which packages the fork owns, so only they are
# formatted, linted and tested.
#
# The fork trusts upstream's CI for upstream's code. What nobody else tests is
# the fork's own delta: the carried patch series and the fork-only files, as
# they sit on the upstream commit this tree merged or rebased onto. So the
# diff base is the merge-base of HEAD with upstream `main`, never fork/main.
# Diffed against fork/main, an upstream sync looks like a change to every
# package (PR #37: 4,673 files), and a run over hundreds of upstream crates
# took 95-105 minutes, past the merge queue's 90-minute timeout (WI-4161).
# The same base serves pull requests and the merge queue: every PR in a queue
# group is fork-owned, so the group's delta is inside the fork-owned delta.
#
# Writes to $GITHUB_OUTPUT:
#   rust   true when any fork-owned file is under codex-rs/ or .github/
#   base   the upstream merge-base commit (the diff base for clippy's gate)
#   list   selected package names, one per line
#   files  the fork-owned files, one per line
#
# Env: FORCED_PACKAGES (comma-separated; overrides the diff, used to measure
# a baseline), FORK_CI_UPSTREAM_URL (default https://github.com/openai/codex.git).
# Run from codex-rs with the workspace version already reset to 0.0.0.
set -euo pipefail
scripts="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="${GITHUB_OUTPUT:?GITHUB_OUTPUT is unset}"

# The fork's `main` is an upstream mirror, but it can lag the commit a sync
# merged (it did: 2351d9e1b against a merge of 6dfaca4dad), and a lagging base
# would put upstream commits back into the fork-owned diff. Fetch upstream.
upstream_url="${FORK_CI_UPSTREAM_URL:-https://github.com/openai/codex.git}"
if ! git remote get-url upstream >/dev/null 2>&1; then
  git remote add upstream "$upstream_url"
fi
git fetch --no-tags --quiet upstream main
base="$(git merge-base HEAD upstream/main)"
echo "base=$base" >> "$out"
echo "upstream merge-base: $base"

mapfile -t files < <(git diff --name-only "$base" HEAD)

# Path scope: a delta with nothing under codex-rs/ or .github/ (docs, FORK.md,
# sdk/) skips the Rust steps. The required check still reports.
rust=false
for f in "${files[@]}"; do
  case "$f" in codex-rs/*|.github/*) rust=true ;; esac
done
[[ -n "${FORCED_PACKAGES:-}" ]] && rust=true
echo "rust=$rust" >> "$out"

emit() {
  # $1 name, then lines on stdin
  {
    echo "$1<<EOF"
    cat
    echo "EOF"
  } >> "$out"
}

if [[ -n "${FORCED_PACKAGES:-}" ]]; then
  mapfile -t packages < <(echo "$FORCED_PACKAGES" | tr ',' '\n')
  echo "forced: ${packages[*]}"
  printf '%s\n' "${packages[@]}" | emit list
  printf '%s\n' "${files[@]:-}" | emit files
  exit 0
fi

# The fork restamps the workspace version (0.0.0 upstream, the release version
# here) in Cargo.toml and, for every member, in Cargo.lock. That stamp is not
# a change to any package, and the workflow has already reset the manifest to
# 0.0.0 before this runs. So the workspace manifest counts only when it
# differs beyond its version line, and the lock is compared by resolve below,
# with both sides at 0.0.0.
all=false
lock_changed=false
others=()
for f in "${files[@]}"; do
  case "$f" in
    codex-rs/Cargo.toml)
      if diff -q <(git show "$base:codex-rs/Cargo.toml" | grep -v '^version = ') \
                 <(git show HEAD:codex-rs/Cargo.toml | grep -v '^version = ') >/dev/null; then
        echo "codex-rs/Cargo.toml: only the version stamp differs; not a package change"
      else
        all=true
      fi
      ;;
    codex-rs/Cargo.lock) lock_changed=true ;;
    *) others+=("$f") ;;
  esac
done

if $all; then
  mapfile -t packages < <(cargo metadata --no-deps --format-version 1 | jq -r '.packages[].name' | sort -u)
else
  mapfile -t packages < <(cargo metadata --no-deps --format-version 1 \
    | python3 "$scripts/fork-changed-packages.py" "${others[@]}")
  if $lock_changed; then
    # Only members whose resolved dependency set moved are selected, by
    # diffing the full `cargo metadata` resolve on the base lock against
    # HEAD's. Both reads are --offline, not --locked: the worktree manifests
    # are at 0.0.0 while HEAD's lock carries the fork's release stamp, so
    # --locked refuses HEAD, and the base lock describes the base manifests.
    # --offline keeps cargo from moving any version; the lock on disk is put
    # back either way.
    base_meta="$(mktemp)"
    head_meta="$(mktemp)"
    # Both reads below are --offline, and reading a manifest graph needs every
    # dependency's own Cargo.toml on disk, so download them first. Not
    # --locked: this rewrites the lock's member stamps to 0.0.0 in place.
    cargo fetch --quiet
    cp Cargo.lock /tmp/Cargo.lock.head
    # A failed read must not leave the base lock checked out: every later
    # cargo call would compile against it.
    trap 'cp /tmp/Cargo.lock.head Cargo.lock' EXIT
    cargo metadata --format-version 1 --offline > "$head_meta"
    git show "$base:codex-rs/Cargo.lock" > Cargo.lock
    cargo metadata --format-version 1 --offline > "$base_meta"
    cp /tmp/Cargo.lock.head Cargo.lock
    trap - EXIT
    mapfile -t locked < <(python3 "$scripts/fork-lock-affected.py" "$base_meta" "$head_meta")
    echo "Cargo.lock: resolve diff selects ${locked[*]:-nothing}"
    # The fork's lock hunks are the version stamp plus the dependencies its
    # patches add, which the patched packages' own files already select. An
    # empty resolve diff is therefore expected here, not a fail-open.
    packages+=("${locked[@]}")
  fi
fi

# Unique, stable order.
mapfile -t packages < <(printf '%s\n' "${packages[@]:-}" | awk 'NF && !seen[$0]++')
# codex-voice-host is the only package that pulls in GStreamer, and it needs
# gstreamer-1.0 >= 1.28 while Ubuntu 24.04 ships 1.24, so no apt package
# satisfies it. Drop it here, or nextest builds it through its -p list and the
# whole job fails on glib-2.0.pc.
mapfile -t packages < <(printf '%s\n' "${packages[@]:-}" | awk 'NF && $0 != "codex-voice-host"')
echo "selected (${#packages[@]}): ${packages[*]:-none}"
printf '%s\n' "${packages[@]:-}" | emit list
printf '%s\n' "${files[@]:-}" | emit files

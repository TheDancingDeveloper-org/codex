#!/usr/bin/env bash
# fork-ci's test run, split in two halves that run as parallel jobs.
#
#   fork-ci-nextest.sh app-server  codex-app-server's integration binaries
#   fork-ci-nextest.sh rest        every other selected test
#
# codex-rs/.config/nextest.toml puts `package(codex-app-server) & kind(test)`
# in the app_server_integration group (max-threads = 1): each case spawns its
# own app-server. Run in one nextest process with everything else, that group
# was a ~25 minute serial tail after the other ~22,000 tests had finished
# (fork-ci run 43201899230). The two halves partition the same test set: the
# same filter selects one half and is negated for the other, so no test is
# dropped, and the group stays serial within its own half.
#
# Reads the selected packages, one per line, from $PACKAGES. Run from codex-rs.
set -euo pipefail
mode="${1:?usage: fork-ci-nextest.sh app-server|rest}"
app_server='package(codex-app-server) & kind(test)'

mapfile -t crates < <(printf '%s\n' "${PACKAGES:-}" | sed '/^$/d')
if [[ "$mode" == "app-server" ]]; then
  # Only this package's integration binaries; nothing to do if it is not selected.
  printf '%s\n' "${crates[@]}" | grep -qx codex-app-server || { echo "codex-app-server not selected"; exit 0; }
  crates=(codex-app-server)
  half="($app_server)"
elif [[ "$mode" == "rest" ]]; then
  half="not ($app_server)"
else
  echo "unknown mode: $mode" >&2
  exit 2
fi
(( ${#crates[@]} )) || exit 0

jobs="$("${GITHUB_WORKSPACE}/.github/scripts/fork-ci-build-jobs.sh")"

# Tests resolve helper binaries in other packages through cargo_bin(), and
# `cargo test -p` never builds another package's bins, so CARGO_BIN_EXE_* is
# unset and the lookup in target/debug misses. Upstream's Bazel build declares
# them as data deps. Build every workspace bin once first. codex-voice-host is
# excluded because it needs GStreamer >= 1.28, which Ubuntu 24.04 lacks.
cargo build --workspace --bins --exclude codex-voice-host --jobs "$jobs"

# nextest UNIONS multiple -E filtersets, so one `not` per line would
# re-include every excluded test as soon as the file has two lines. Build a
# single expression: this half, minus the known failures.
known=()
while IFS= read -r line; do
  line="${line%%#*}"
  line="${line#"${line%%[![:space:]]*}"}"
  [[ -z "$line" ]] && continue
  known+=("(${line})")
done < "${GITHUB_WORKSPACE}/.github/fork-ci-known-failures.txt"
expr="$half"
if (( ${#known[@]} > 0 )); then
  joined="$(printf '%s | ' "${known[@]}")"
  expr="($half) & not (${joined% | })"
fi
echo "filter: $expr"

args=()
for crate in "${crates[@]}"; do args+=(-p "$crate"); done
# A failing crate must not hide the rest: --no-fail-fast keeps nextest running
# after the first failure. One invocation for every package: per-crate loops
# re-resolve features and rebuild shared dependencies once per crate.
# --timings records the build's critical path.
status=0
cargo nextest run "${args[@]}" --no-fail-fast --build-jobs "$jobs" --timings -E "$expr" || status=$?
mkdir -p /tmp/cargo-timings
[[ -f cargo-timing.html ]] && cp cargo-timing.html "/tmp/cargo-timings/nextest-$mode-timing.html"
# nextest exits 4 when the filter matched no tests (e.g. every app-server test
# is a known failure); that is not a failure of this half.
if (( status == 4 )); then
  echo "no tests matched in this half"
  status=0
fi
exit "$status"

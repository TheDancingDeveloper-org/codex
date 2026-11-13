#!/usr/bin/env bash
# Offline test of fork-ci-build-jobs.sh against fake cgroup files.
#   bash .github/scripts/fork-ci-build-jobs-test.sh
set -euo pipefail
script="$(dirname "$0")/fork-ci-build-jobs.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
gib=$((1024 * 1024 * 1024))
printf 'MemTotal: %d kB\n' $((64 * 1024 * 1024)) > "$tmp/meminfo"
fail=0

# check NAME NPROC MODE EXPECTED; the cgroup files are set up before each call.
check() {
  local got
  got="$(FORK_CI_CGROUP_ROOT="$tmp/cg" FORK_CI_MEMINFO="$tmp/meminfo" \
    FORK_CI_NPROC="$2" bash "$script" "$3" 2>/dev/null)"
  if [[ "$got" == "$4" ]]; then echo "ok   $1 -> $got"; else echo "FAIL $1: got $got, want $4"; fail=1; fi
}
v2() { rm -rf "$tmp/cg"; mkdir -p "$tmp/cg"; echo "$1" > "$tmp/cg/cpu.max"; echo "$2" > "$tmp/cg/memory.max"; }
v1() { rm -rf "$tmp/cg"; mkdir -p "$tmp/cg/cpu" "$tmp/cg/memory"
  echo "$1" > "$tmp/cg/cpu/cpu.cfs_quota_us"; echo 100000 > "$tmp/cg/cpu/cpu.cfs_period_us"
  echo "$2" > "$tmp/cg/memory/memory.limit_in_bytes"; }

v2 "800000 100000" $((24 * gib)); check "24GiB/8cpu quota on 32 cores" 32 build 5
v2 "200000 100000" $((24 * gib)); check "v2 quota 2 cpu caps width" 32 build 2
v2 "150000 100000" $((24 * gib)); check "v2 quota 1.5 cpu rounds up" 32 build 2
v2 "max 100000" $((24 * gib));    check "nodef-rust 24GiB/14 cores" 14 build 5
v2 "max 100000" $((10 * gib));    check "nodef-docker 10GiB build" 14 build 2
v2 "max 100000" $((10 * gib));    check "nodef-docker 10GiB integration" 14 integration 1
v2 "100000 100000" $((10 * gib)); check "10GiB with 1 cpu quota" 14 build 1
v2 "max 100000" max;              check "no limits: MemTotal 64GiB, 4 cores" 4 build 4
v2 "max 100000" $((1 * gib));     check "tiny memory floors at 1" 8 build 1
v1 300000 $((24 * gib));          check "v1 quota 3 cpu" 32 build 3
v1 -1 $((24 * gib));              check "v1 no quota" 32 build 5
exit "$fail"

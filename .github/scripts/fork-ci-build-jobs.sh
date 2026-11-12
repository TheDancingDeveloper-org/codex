#!/usr/bin/env bash
# Prints how many rustc processes fork-ci may run at once on this runner.
#
# Not a fixed cap: every core is used unless the job's memory limit cannot
# hold that many rustc at once. At full width the OOM killer took a compile
# process (WI-3954, fork-ci runs 43003880297 and 43136846045). A 3 GiB budget
# per rustc was still too small: on nodef-gha-public-docker (10 GiB cgroup;
# the rust-heavy,node-f labels also match the 24 GiB nodef-gha-public-rust)
# it gave 3 jobs and codex-core's lib test was killed (run 43311746966,
# WI-4161). The kill shows only as a silent `kache ... (exit status: 1)`.
# So jobs = min(cores, (memory limit - 2 GiB for cargo, kache and the
# linker) / 4 GiB), at least 1: 2 on the 10 GiB runner, as the old fixed
# --jobs 2 that passed there, and 5 on the 24 GiB one. On a runner with no
# cgroup limit the host's MemTotal is the limit.
#
# Cores are min(nproc, the cgroup CPU quota rounded up): ci-bigmem-0 has an
# 8-CPU limit but nproc reports the host's cores. The quota comes from
# cpu.max on cgroup v2 ("<quota> <period>" or "max <period>") or
# cpu.cfs_quota_us / cpu.cfs_period_us on v1 (-1 = none). So 24 GiB / 8 CPU
# gives 5, 24 GiB / 14 cores 5, 10 GiB 2 (integration 1).
#
# FORK_CI_CGROUP_ROOT, FORK_CI_MEMINFO and FORK_CI_NPROC override the
# inputs for fork-ci-build-jobs-test.sh.
#
#   fork-ci-build-jobs.sh               width for an ordinary build
#   fork-ci-build-jobs.sh integration   width for the integration test
#                                       binaries: 1 below 16 GiB
#
# The integration width exists because even 2 rustc at once OOMed on the
# 10 GiB runner while compiling and linking codex-core's `all` test binary
# (run 43317835444, 2 OOM kills). Below 16 GiB those binaries build one at a
# time; at or above it the ordinary width applies.
set -euo pipefail
mode="${1:-build}"
small_kib=$((16 * 1024 * 1024))
per_job_kib=$((4 * 1024 * 1024))
reserve_kib=$((2 * 1024 * 1024))
cg="${FORK_CI_CGROUP_ROOT:-/sys/fs/cgroup}"
nproc_cores="${FORK_CI_NPROC:-$(nproc)}"
cores="$nproc_cores"
quota="none"
if [[ -r "$cg/cpu.max" ]]; then
  read -r q p < "$cg/cpu.max"
  [[ "$q" != "max" ]] && quota="$q/$p"
elif [[ -r "$cg/cpu/cpu.cfs_quota_us" && -r "$cg/cpu/cpu.cfs_period_us" ]]; then
  q="$(cat "$cg/cpu/cpu.cfs_quota_us")"
  p="$(cat "$cg/cpu/cpu.cfs_period_us")"
  (( q > 0 )) && quota="$q/$p"
fi
if [[ "$quota" != "none" ]]; then
  q="${quota%/*}" p="${quota#*/}"
  quota_cores=$(((q + p - 1) / p))
  (( quota_cores < cores )) && cores="$quota_cores"
fi
limit_kib=""
if [[ -r "$cg/memory.max" ]]; then
  max="$(cat "$cg/memory.max")"
  [[ "$max" != "max" ]] && limit_kib=$((max / 1024))
elif [[ -r "$cg/memory/memory.limit_in_bytes" ]]; then
  limit_kib=$(( $(cat "$cg/memory/memory.limit_in_bytes") / 1024 ))
fi
total_kib="$(awk '/^MemTotal:/ {print $2}' "${FORK_CI_MEMINFO:-/proc/meminfo}")"
if [[ -z "$limit_kib" ]] || (( limit_kib > total_kib )); then
  limit_kib="$total_kib"
fi
jobs=$(((limit_kib - reserve_kib) / per_job_kib))
(( jobs > cores )) && jobs="$cores"
(( jobs < 1 )) && jobs=1
reason="min(cores, (limit - 2 GiB) / 4 GiB)"
if [[ "$mode" == "integration" ]] && (( limit_kib < small_kib )); then
  jobs=1
  reason="integration test binaries below 16 GiB build one at a time"
fi
echo "cores=$cores (nproc=$nproc_cores cpu_quota=$quota) memory_limit_gib=$((limit_kib / 1024 / 1024)) mode=$mode build_jobs=$jobs ($reason)" >&2
echo "$jobs"

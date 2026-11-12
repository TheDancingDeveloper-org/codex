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
set -euo pipefail
per_job_kib=$((4 * 1024 * 1024))
reserve_kib=$((2 * 1024 * 1024))
cores="$(nproc)"
limit_kib=""
if [[ -r /sys/fs/cgroup/memory.max ]]; then
  max="$(cat /sys/fs/cgroup/memory.max)"
  [[ "$max" != "max" ]] && limit_kib=$((max / 1024))
elif [[ -r /sys/fs/cgroup/memory/memory.limit_in_bytes ]]; then
  limit_kib=$(( $(cat /sys/fs/cgroup/memory/memory.limit_in_bytes) / 1024 ))
fi
total_kib="$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)"
if [[ -z "$limit_kib" ]] || (( limit_kib > total_kib )); then
  limit_kib="$total_kib"
fi
jobs=$(((limit_kib - reserve_kib) / per_job_kib))
(( jobs > cores )) && jobs="$cores"
(( jobs < 1 )) && jobs=1
echo "cores=$cores memory_limit_gib=$((limit_kib / 1024 / 1024)) build_jobs=$jobs" >&2
echo "$jobs"

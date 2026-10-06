//! What a model server actually uses, as opposed to what its spec declares.
//!
//! Information only: the numbers are reported (admin API, metrics, a one-off
//! warning when a server outgrows its declaration) and never feed back into
//! reservations or eviction. Whether they should is a design decision left for
//! later; reporting first shows how far specs drift in practice.
//!
//! Host memory is the resident set summed over the server's process group,
//! because servers such as vLLM fork engine workers that hold most of the
//! memory. GPU memory is what `nvidia-smi` attributes to any pid in that group.

use std::collections::{HashMap, HashSet};
use std::process::Stdio;

/// Usage of one process group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Usage {
    /// Resident host memory, MiB. `None` if the group has no live process.
    pub mem_mb: Option<u64>,
    /// VRAM, MiB. `None` when there is no `nvidia-smi` to ask.
    pub gpu_mb: Option<u64>,
}

/// Measure the process groups led by `pids`. One `/proc` scan and one
/// `nvidia-smi` call for all of them.
pub async fn measure(pids: &[u32]) -> HashMap<u32, Usage> {
    if pids.is_empty() {
        return HashMap::new();
    }
    let wanted: HashSet<u32> = pids.iter().copied().collect();
    // Hundreds of small file reads: keep them off the async workers.
    let groups = tokio::task::spawn_blocking(move || scan_groups(&wanted))
        .await
        .unwrap_or_default();
    let gpu_apps = read_compute_apps().await;

    pids.iter()
        .map(|pgid| {
            let group = groups.get(pgid);
            let mem_mb = group
                .filter(|g| !g.members.is_empty())
                .map(|g| g.rss_kb / 1024);
            let gpu_mb = gpu_apps.as_ref().map(|apps| {
                let members: HashSet<u32> = group
                    .map(|g| g.members.iter().copied().collect())
                    .unwrap_or_default();
                apps.iter()
                    .filter(|(pid, _)| members.contains(pid))
                    .map(|(_, mb)| mb)
                    .sum()
            });
            (*pgid, Usage { mem_mb, gpu_mb })
        })
        .collect()
}

#[derive(Debug, Default)]
struct Group {
    rss_kb: u64,
    members: Vec<u32>,
}

/// Every live process whose group is one of `wanted`, with its resident set.
fn scan_groups(wanted: &HashSet<u32>) -> HashMap<u32, Group> {
    let mut groups: HashMap<u32, Group> = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return groups;
    };
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        // A process can exit between the listing and the read: skip it.
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            continue;
        };
        let Some(pgrp) = parse_stat_pgrp(&stat).filter(|g| wanted.contains(g)) else {
            continue;
        };
        let group = groups.entry(pgrp).or_default();
        group.members.push(pid);
        if let Some(kb) = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .ok()
            .and_then(|status| parse_vmrss_kb(&status))
        {
            group.rss_kb += kb;
        }
    }
    groups
}

/// The process group from `/proc/{pid}/stat`: `pid (comm) state ppid pgrp ..`.
/// The command name can hold spaces and parentheses, so parse from the last
/// closing parenthesis.
fn parse_stat_pgrp(stat: &str) -> Option<u32> {
    let after_comm = &stat[stat.rfind(')')? + 1..];
    after_comm.split_whitespace().nth(2)?.parse().ok()
}

/// `VmRSS` from `/proc/{pid}/status`, in KiB. Kernel threads have none.
fn parse_vmrss_kb(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// `(pid, MiB)` per compute process, or `None` without `nvidia-smi`.
async fn read_compute_apps() -> Option<Vec<(u32, u64)>> {
    let output = tokio::process::Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=pid,used_memory",
            "--format=csv,noheader,nounits",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(parse_compute_apps(&String::from_utf8_lossy(&output.stdout)))
}

/// Lines like `12345, 2048`. A process can be listed twice (once per GPU), so
/// the caller sums. `[N/A]` and other junk lines are skipped.
fn parse_compute_apps(text: &str) -> Vec<(u32, u64)> {
    text.lines()
        .filter_map(|line| {
            let (pid, mb) = line.split_once(',')?;
            Some((pid.trim().parse().ok()?, mb.trim().parse().ok()?))
        })
        .collect()
}

/// Whether a measurement outgrew its declaration enough to be worth a
/// warning: more than 10% over and at least 256 MiB over, so rounding and
/// small specs do not warn.
pub fn is_over_budget(declared_mb: u64, measured_mb: u64) -> bool {
    measured_mb > declared_mb + declared_mb / 10 && measured_mb >= declared_mb + 256
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_group_survives_awkward_command_names() {
        assert_eq!(
            parse_stat_pgrp("42 (python3) S 1 4242 4242 0 -1 4194560"),
            Some(4242)
        );
        assert_eq!(
            parse_stat_pgrp("42 (a) b (c) S 1 777 777 0 -1"),
            Some(777),
            "the name is everything up to the last parenthesis"
        );
        assert_eq!(parse_stat_pgrp("garbage"), None);
        assert_eq!(parse_stat_pgrp("1 (x) S"), None);
    }

    #[test]
    fn vmrss_is_read_in_kib() {
        let status = "Name:\tpython3\nVmPeak:\t  200000 kB\nVmRSS:\t   12345 kB\nThreads:\t1\n";
        assert_eq!(parse_vmrss_kb(status), Some(12345));
        assert_eq!(
            parse_vmrss_kb("Name:\tkthreadd\nThreads:\t1\n"),
            None,
            "kernel threads have no resident set"
        );
    }

    #[test]
    fn compute_apps_parse_and_skip_junk() {
        let apps =
            parse_compute_apps("1234, 2048\n5678, 512\n99, [N/A]\n\nnot a line\n1234, 100\n");
        assert_eq!(apps, vec![(1234, 2048), (5678, 512), (1234, 100)]);
        assert!(parse_compute_apps("").is_empty(), "no compute apps");
    }

    #[test]
    fn over_budget_needs_both_ten_percent_and_256_mib() {
        // 10% of 10000 is 1000: 10500 is only 5% over, though 500 MiB.
        assert!(!is_over_budget(10_000, 10_500));
        assert!(is_over_budget(10_000, 11_001));
        // 100 -> 300 is 200% over but only 200 MiB.
        assert!(!is_over_budget(100, 300));
        assert!(is_over_budget(100, 356));
        assert!(!is_over_budget(100, 90), "under budget is fine");
    }

    #[tokio::test]
    async fn this_process_measures_as_its_own_group_member() {
        // Our own pid leads no group under cargo, but it is a member of one:
        // scan by our own process group instead.
        let pgrp = parse_stat_pgrp(&std::fs::read_to_string("/proc/self/stat").unwrap()).unwrap();
        let usage = measure(&[pgrp]).await;
        assert!(usage[&pgrp].mem_mb.unwrap() > 0);
        let gone = measure(&[u32::MAX - 1]).await;
        assert_eq!(gone[&(u32::MAX - 1)].mem_mb, None);
    }
}

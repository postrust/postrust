//! The resource budget: what is there, what is promised, and whether a model
//! fits.
//!
//! Two sources, deliberately combined:
//!
//! - **Accounting.** Every instance reserves what its spec declares, and the
//!   sum stays under the budget. This is the primary rule because it is
//!   deterministic: it does not depend on when a model server happens to
//!   allocate.
//! - **Probe.** `/proc/meminfo` and `nvidia-smi`, read periodically. With
//!   `respect_probe`, a load also has to fit what is actually free, minus
//!   reservations that have not shown up in the probe yet. That catches memory
//!   taken by processes this node does not manage, and specs that understate.

use std::process::Stdio;

use serde::Serialize;

use crate::config::{GpuConfig, ResourceConfig};

/// What a model needs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Need {
    /// Host memory, MiB.
    pub mem_mb: u64,
    /// VRAM on one GPU, MiB. 0: no GPU.
    pub gpu_mb: u64,
}

/// What an instance holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Reservation {
    /// Host memory, MiB.
    pub mem_mb: u64,
    /// GPU index and VRAM, MiB.
    pub gpu: Option<(u32, u64)>,
    /// Whether the probe can be expected to see it yet. False while loading.
    pub materialized: bool,
}

/// What a load is short of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Shortfall {
    /// Host memory, MiB.
    pub mem_mb: u64,
    /// VRAM on the GPU closest to fitting, MiB.
    pub gpu_mb: u64,
}

impl Shortfall {
    /// Nothing is short.
    pub fn is_zero(&self) -> bool {
        self.mem_mb == 0 && self.gpu_mb == 0
    }
}

impl std::fmt::Display for Shortfall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "short {} MiB host memory, {} MiB VRAM",
            self.mem_mb, self.gpu_mb
        )
    }
}

/// One probe reading.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Probe {
    /// Total host memory, MiB.
    pub mem_total_mb: u64,
    /// Available host memory, MiB.
    pub mem_available_mb: u64,
    /// GPUs.
    pub gpus: Vec<GpuProbe>,
}

/// One GPU's reading.
#[derive(Debug, Clone, Serialize)]
pub struct GpuProbe {
    /// CUDA index.
    pub index: u32,
    /// Name.
    pub name: String,
    /// Total VRAM, MiB.
    pub total_mb: u64,
    /// Used VRAM, MiB.
    pub used_mb: u64,
}

impl Probe {
    /// Read the machine. Missing sources read as absent, not as an error: a
    /// CPU-only box has no `nvidia-smi`.
    pub async fn read() -> Probe {
        let mut probe = Probe::default();
        if let Ok(text) = tokio::fs::read_to_string("/proc/meminfo").await {
            let (total, available) = parse_meminfo(&text);
            probe.mem_total_mb = total;
            probe.mem_available_mb = available;
        }
        probe.gpus = read_nvidia_smi().await.unwrap_or_default();
        probe
    }
}

fn parse_meminfo(text: &str) -> (u64, u64) {
    let field = |name: &str| {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<u64>().ok())
            .map(|kb| kb / 1024)
            .unwrap_or(0)
    };
    (field("MemTotal:"), field("MemAvailable:"))
}

async fn read_nvidia_smi() -> Option<Vec<GpuProbe>> {
    let output = tokio::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,memory.total,memory.used",
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
    Some(parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_nvidia_smi(text: &str) -> Vec<GpuProbe> {
    text.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(',').map(str::trim).collect();
            if parts.len() != 4 {
                return None;
            }
            Some(GpuProbe {
                index: parts[0].parse().ok()?,
                name: parts[1].to_string(),
                total_mb: parts[2].parse().ok()?,
                used_mb: parts[3].parse().ok()?,
            })
        })
        .collect()
}

/// One GPU's budget and reservations.
#[derive(Debug, Clone, Serialize)]
pub struct GpuSlot {
    /// CUDA index.
    pub index: u32,
    /// Budget, MiB.
    pub budget_mb: u64,
    /// Reserved, MiB.
    pub reserved_mb: u64,
    /// Reserved but not yet visible to the probe, MiB.
    pub pending_mb: u64,
}

impl GpuSlot {
    fn free(&self) -> u64 {
        self.budget_mb.saturating_sub(self.reserved_mb)
    }
}

/// The node's budget and what is reserved against it.
#[derive(Debug, Clone, Serialize)]
pub struct ResourcePool {
    /// Host memory budget, MiB.
    pub mem_budget_mb: u64,
    /// Host memory reserved, MiB.
    pub mem_reserved_mb: u64,
    /// Host memory reserved but not yet visible to the probe, MiB.
    pub mem_pending_mb: u64,
    /// GPUs.
    pub gpus: Vec<GpuSlot>,
    /// Last probe.
    pub probe: Probe,
    /// Whether the probe constrains loads.
    pub respect_probe: bool,
    gpu_reserve_mb: u64,
}

impl ResourcePool {
    /// Size the budget from the config, falling back to the probe.
    pub fn new(config: &ResourceConfig, probe: Probe) -> Self {
        let mem_budget_mb = config
            .memory_mb
            .unwrap_or_else(|| probe.mem_total_mb.saturating_sub(config.memory_reserve_mb));
        let gpus = match &config.gpus {
            Some(declared) => declared
                .iter()
                .map(|g: &GpuConfig| GpuSlot {
                    index: g.index,
                    budget_mb: g.vram_mb,
                    reserved_mb: 0,
                    pending_mb: 0,
                })
                .collect(),
            None => probe
                .gpus
                .iter()
                .map(|g| GpuSlot {
                    index: g.index,
                    budget_mb: g.total_mb.saturating_sub(config.gpu_reserve_mb),
                    reserved_mb: 0,
                    pending_mb: 0,
                })
                .collect(),
        };
        Self {
            mem_budget_mb,
            mem_reserved_mb: 0,
            mem_pending_mb: 0,
            gpus,
            probe,
            respect_probe: config.respect_probe,
            gpu_reserve_mb: config.gpu_reserve_mb,
        }
    }

    /// Replace the probe reading.
    pub fn update_probe(&mut self, probe: Probe) {
        self.probe = probe;
    }

    /// Host memory free by both rules.
    pub fn mem_free(&self) -> u64 {
        let accounted = self.mem_budget_mb.saturating_sub(self.mem_reserved_mb);
        if !self.respect_probe || self.probe.mem_total_mb == 0 {
            return accounted;
        }
        accounted.min(
            self.probe
                .mem_available_mb
                .saturating_sub(self.mem_pending_mb),
        )
    }

    /// VRAM free on one GPU by both rules.
    pub fn gpu_free(&self, slot: &GpuSlot) -> u64 {
        let accounted = slot.free();
        if !self.respect_probe {
            return accounted;
        }
        match self.probe.gpus.iter().find(|g| g.index == slot.index) {
            Some(g) => accounted.min(
                g.total_mb
                    .saturating_sub(g.used_mb)
                    .saturating_sub(self.gpu_reserve_mb)
                    .saturating_sub(slot.pending_mb),
            ),
            None => accounted,
        }
    }

    /// What `need` is short of, if anything.
    pub fn shortfall(&self, need: Need) -> Shortfall {
        let mem_mb = need.mem_mb.saturating_sub(self.mem_free());
        let gpu_mb = if need.gpu_mb == 0 {
            0
        } else if self.gpus.is_empty() {
            // No GPU at all: nothing evicted will ever make room.
            u64::MAX
        } else {
            self.gpus
                .iter()
                .map(|g| need.gpu_mb.saturating_sub(self.gpu_free(g)))
                .min()
                .unwrap_or(u64::MAX)
        };
        Shortfall { mem_mb, gpu_mb }
    }

    /// Whether `need` can ever fit, with nothing else loaded.
    pub fn could_ever_fit(&self, need: Need) -> bool {
        need.mem_mb <= self.mem_budget_mb
            && (need.gpu_mb == 0 || self.gpus.iter().any(|g| need.gpu_mb <= g.budget_mb))
    }

    /// Reserve `need`, choosing the GPU it fits most tightly, so large free
    /// regions stay available for large models.
    pub fn try_reserve(&mut self, need: Need) -> Option<Reservation> {
        if !self.shortfall(need).is_zero() {
            return None;
        }
        let gpu = if need.gpu_mb == 0 {
            None
        } else {
            let index = self
                .gpus
                .iter()
                .filter(|g| self.gpu_free(g) >= need.gpu_mb)
                .min_by_key(|g| self.gpu_free(g))?
                .index;
            let slot = self.gpus.iter_mut().find(|g| g.index == index)?;
            slot.reserved_mb += need.gpu_mb;
            slot.pending_mb += need.gpu_mb;
            Some((index, need.gpu_mb))
        };
        self.mem_reserved_mb += need.mem_mb;
        self.mem_pending_mb += need.mem_mb;
        Some(Reservation {
            mem_mb: need.mem_mb,
            gpu,
            materialized: false,
        })
    }

    /// The instance is up, so the probe now sees what it holds.
    pub fn materialize(&mut self, reservation: &mut Reservation) {
        if reservation.materialized {
            return;
        }
        reservation.materialized = true;
        self.mem_pending_mb = self.mem_pending_mb.saturating_sub(reservation.mem_mb);
        if let Some((index, mb)) = reservation.gpu {
            if let Some(slot) = self.gpus.iter_mut().find(|g| g.index == index) {
                slot.pending_mb = slot.pending_mb.saturating_sub(mb);
            }
        }
    }

    /// Give a reservation back.
    pub fn release(&mut self, reservation: &Reservation) {
        self.mem_reserved_mb = self.mem_reserved_mb.saturating_sub(reservation.mem_mb);
        if !reservation.materialized {
            self.mem_pending_mb = self.mem_pending_mb.saturating_sub(reservation.mem_mb);
        }
        if let Some((index, mb)) = reservation.gpu {
            if let Some(slot) = self.gpus.iter_mut().find(|g| g.index == index) {
                slot.reserved_mb = slot.reserved_mb.saturating_sub(mb);
                if !reservation.materialized {
                    slot.pending_mb = slot.pending_mb.saturating_sub(mb);
                }
            }
        }
    }

    /// Move a reservation's VRAM to host memory, as parking does: the weights
    /// leave the GPU and sit in RAM. Returns false, changing nothing, if host
    /// memory cannot take them.
    pub fn park(&mut self, reservation: &mut Reservation) -> bool {
        let Some((index, mb)) = reservation.gpu else {
            return false;
        };
        if self.mem_budget_mb.saturating_sub(self.mem_reserved_mb) < mb {
            return false;
        }
        if let Some(slot) = self.gpus.iter_mut().find(|g| g.index == index) {
            slot.reserved_mb = slot.reserved_mb.saturating_sub(mb);
        }
        self.mem_reserved_mb += mb;
        reservation.mem_mb += mb;
        reservation.gpu = None;
        true
    }

    /// Undo [`park`](Self::park): put `gpu_mb` back on a GPU and take it off
    /// host memory. Any GPU will do; the server is told which on wake only
    /// through `CUDA_VISIBLE_DEVICES` at start, so in practice the same one
    /// must be free, and this asks for exactly that index.
    pub fn unpark(&mut self, reservation: &mut Reservation, index: u32, gpu_mb: u64) -> bool {
        let Some(slot) = self.gpus.iter().find(|g| g.index == index) else {
            return false;
        };
        if self.gpu_free(slot) < gpu_mb {
            return false;
        }
        let slot = self
            .gpus
            .iter_mut()
            .find(|g| g.index == index)
            .expect("found above");
        slot.reserved_mb += gpu_mb;
        self.mem_reserved_mb = self.mem_reserved_mb.saturating_sub(gpu_mb);
        reservation.mem_mb = reservation.mem_mb.saturating_sub(gpu_mb);
        reservation.gpu = Some((index, gpu_mb));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(mem: u64, gpus: &[u64]) -> ResourcePool {
        let config = ResourceConfig {
            memory_mb: Some(mem),
            gpus: Some(
                gpus.iter()
                    .enumerate()
                    .map(|(i, mb)| GpuConfig::new(i as u32, *mb))
                    .collect(),
            ),
            respect_probe: false,
            ..Default::default()
        };
        ResourcePool::new(&config, Probe::default())
    }

    #[test]
    fn meminfo_is_read_in_mib() {
        let text = "MemTotal:       16384000 kB\nMemFree: 1 kB\nMemAvailable:    8192000 kB\n";
        assert_eq!(parse_meminfo(text), (16000, 8000));
    }

    #[test]
    fn nvidia_smi_lines_parse() {
        let gpus = parse_nvidia_smi("0, NVIDIA RTX 4050, 6141, 12\n1, A100, 81920, 0\n");
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[1].total_mb, 81920);
    }

    #[test]
    fn reserve_picks_the_tightest_gpu() {
        let mut p = pool(64_000, &[24_000, 8_000]);
        let r = p
            .try_reserve(Need {
                mem_mb: 1000,
                gpu_mb: 6000,
            })
            .unwrap();
        assert_eq!(r.gpu, Some((1, 6000)));
        // The big GPU is still whole for a big model.
        assert!(p
            .try_reserve(Need {
                mem_mb: 1000,
                gpu_mb: 20_000
            })
            .is_some());
    }

    #[test]
    fn shortfall_reports_what_is_missing() {
        let mut p = pool(10_000, &[8_000]);
        p.try_reserve(Need {
            mem_mb: 6000,
            gpu_mb: 6000,
        })
        .unwrap();
        let short = p.shortfall(Need {
            mem_mb: 6000,
            gpu_mb: 4000,
        });
        assert_eq!(
            short,
            Shortfall {
                mem_mb: 2000,
                gpu_mb: 2000
            }
        );
    }

    #[test]
    fn release_returns_everything() {
        let mut p = pool(10_000, &[8_000]);
        let r = p
            .try_reserve(Need {
                mem_mb: 6000,
                gpu_mb: 6000,
            })
            .unwrap();
        p.release(&r);
        assert_eq!(p.mem_reserved_mb, 0);
        assert_eq!(p.gpus[0].reserved_mb, 0);
        assert_eq!(p.mem_pending_mb, 0);
    }

    #[test]
    fn park_moves_vram_to_host_memory_and_back() {
        let mut p = pool(20_000, &[8_000]);
        let mut r = p
            .try_reserve(Need {
                mem_mb: 2000,
                gpu_mb: 6000,
            })
            .unwrap();
        p.materialize(&mut r);
        assert!(p.park(&mut r));
        assert_eq!(p.gpus[0].reserved_mb, 0);
        assert_eq!(p.mem_reserved_mb, 8000);
        assert!(p.unpark(&mut r, 0, 6000));
        assert_eq!(p.gpus[0].reserved_mb, 6000);
        assert_eq!(p.mem_reserved_mb, 2000);
    }

    #[test]
    fn a_gpu_model_never_fits_a_cpu_node() {
        let p = pool(64_000, &[]);
        assert!(!p.could_ever_fit(Need {
            mem_mb: 1,
            gpu_mb: 1
        }));
        assert_eq!(
            p.shortfall(Need {
                mem_mb: 1,
                gpu_mb: 1
            })
            .gpu_mb,
            u64::MAX
        );
    }

    #[test]
    fn probe_constrains_when_respected() {
        let mut p = pool(64_000, &[]);
        p.respect_probe = true;
        p.update_probe(Probe {
            mem_total_mb: 64_000,
            mem_available_mb: 3000,
            gpus: vec![],
        });
        assert_eq!(p.mem_free(), 3000);
    }
}

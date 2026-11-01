//! System sampling. Runs on a background thread so slow process enumeration
//! never blocks input handling or rendering.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sysinfo::{
    Disks, MINIMUM_CPU_UPDATE_INTERVAL, Networks, ProcessRefreshKind, ProcessesToUpdate, System,
    Uid, UpdateKind, Users,
};

/// Static facts about the machine, gathered once at startup.
#[derive(Debug, Clone, Default)]
pub struct StaticInfo {
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub cpu_brand: String,
    pub logical_cores: usize,
    pub physical_cores: usize,
}

#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cmd: String,
    pub user: String,
    pub cpu: f32,
    pub mem: u64,
    pub mem_pct: f32,
    pub run_time: u64,
    pub status: String,
    pub threads: usize,
}

#[derive(Debug, Clone)]
pub struct NetIface {
    pub name: String,
    pub rx_rate: u64,
    pub tx_rate: u64,
    pub rx_total: u64,
    pub tx_total: u64,
}

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub used: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub cpu_total: f32,
    pub cpu_cores: Vec<f32>,
    pub cpu_freq_mhz: u64,
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub load: (f64, f64, f64),
    pub uptime: u64,
    pub rx_rate: u64,
    pub tx_rate: u64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub ifaces: Vec<NetIface>,
    pub disks: Vec<DiskInfo>,
    pub procs: Vec<ProcInfo>,
    pub n_running: usize,
    pub n_threads: usize,
    pub sample_ms: u64,
}

/// Commands the UI can send to the sampler thread.
pub enum Command {
    SetInterval(Duration),
    Stop,
}

pub struct Sampler {
    pub info: StaticInfo,
    rx: Receiver<Snapshot>,
    tx: Sender<Command>,
}

impl Sampler {
    /// Spawn the sampling thread. The first snapshot arrives after roughly
    /// one interval, because CPU usage needs two measurements.
    pub fn spawn(interval: Duration) -> Result<Self, sysinfo::Error> {
        let mut sys = System::new()?;
        sys.refresh_cpu_all();
        sys.refresh_memory();

        let info = StaticInfo {
            hostname: System::host_name().unwrap_or_else(|_| "?".into()),
            os: System::long_os_version()
                .or_else(|_| System::name())
                .unwrap_or_else(|_| "unknown OS".into()),
            arch: System::cpu_arch(),
            cpu_brand: sys
                .cpus()
                .first()
                .map(|c| c.brand().trim().to_string())
                .unwrap_or_default(),
            logical_cores: sys.cpus().len(),
            physical_cores: System::physical_core_count().unwrap_or(0),
        };

        let (snap_tx, snap_rx) = std::sync::mpsc::channel();
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();

        thread::Builder::new()
            .name("etop-sampler".into())
            .spawn(move || run(sys, interval, snap_tx, cmd_rx))
            .map_err(sysinfo::Error::Io)?;

        Ok(Self {
            info,
            rx: snap_rx,
            tx: cmd_tx,
        })
    }

    /// Return the newest snapshot, discarding any older ones queued behind it.
    pub fn latest(&self) -> Option<Snapshot> {
        let mut latest = None;
        while let Ok(s) = self.rx.try_recv() {
            latest = Some(s);
        }
        latest
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }
}

fn run(mut sys: System, mut interval: Duration, out: Sender<Snapshot>, cmds: Receiver<Command>) {
    let mut networks = Networks::new_with_refreshed_list().ok();
    let mut disks = Disks::new_with_refreshed_list().ok();
    let users = Users::new_with_refreshed_list().ok();
    let mut user_cache: HashMap<Uid, String> = HashMap::new();

    let proc_kind = ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_user(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_tasks();

    let mut last = Instant::now();

    loop {
        // Drain commands first.
        loop {
            match cmds.try_recv() {
                Ok(Command::Stop) => return,
                Ok(Command::SetInterval(d)) => interval = d,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let started = Instant::now();
        let elapsed = started.duration_since(last).as_secs_f64().max(0.001);
        last = started;

        sys.refresh_cpu_all();
        sys.refresh_memory();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, proc_kind);

        let mut ifaces: Vec<NetIface> = Vec::new();
        if let Some(n) = networks.as_mut() {
            n.refresh(true);
            for (name, data) in &*n {
                // Skip interfaces that have never carried traffic (macOS has
                // dozens of idle utun/awdl/llw devices).
                if data.total_received() == 0 && data.total_transmitted() == 0 {
                    continue;
                }
                ifaces.push(NetIface {
                    name: name.to_string_lossy().into_owned(),
                    rx_rate: (data.received() as f64 / elapsed) as u64,
                    tx_rate: (data.transmitted() as f64 / elapsed) as u64,
                    rx_total: data.total_received(),
                    tx_total: data.total_transmitted(),
                });
            }
        }
        // Busiest first, then alphabetical so idle interfaces keep a stable order.
        ifaces.sort_by(|a, b| {
            (b.rx_rate + b.tx_rate)
                .cmp(&(a.rx_rate + a.tx_rate))
                .then_with(|| a.name.cmp(&b.name))
        });
        let rx_rate = ifaces.iter().map(|i| i.rx_rate).sum();
        let tx_rate = ifaces.iter().map(|i| i.tx_rate).sum();
        let rx_total = ifaces.iter().map(|i| i.rx_total).sum();
        let tx_total = ifaces.iter().map(|i| i.tx_total).sum();

        let disk_list = match disks.as_mut() {
            Some(d) => {
                d.refresh(true);
                collect_disks(d)
            }
            None => Vec::new(),
        };

        let mem_total = sys.total_memory().max(1);
        let mut procs = Vec::with_capacity(sys.processes().len());
        let mut n_running = 0;
        let mut n_threads = 0;
        for (pid, p) in sys.processes() {
            // Skip kernel threads / task entries that sysinfo exposes on Linux.
            if p.thread_kind().is_some() {
                continue;
            }
            let status = p.status().to_string();
            if status.starts_with("Run") {
                n_running += 1;
            }
            let threads = p.tasks().map(|t| t.len()).unwrap_or(0);
            n_threads += threads.max(1);

            let user = match p.user_id() {
                Some(uid) => user_cache
                    .entry(uid.clone())
                    .or_insert_with(|| {
                        users
                            .as_ref()
                            .and_then(|u| u.get_user_by_id(uid))
                            .map(|u| u.name().to_string())
                            .unwrap_or_else(|| uid.to_string())
                    })
                    .clone(),
                None => "?".into(),
            };

            let cmd = if p.cmd().is_empty() {
                p.name().to_string_lossy().into_owned()
            } else {
                p.cmd()
                    .iter()
                    .map(|c| c.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" ")
            };

            procs.push(ProcInfo {
                pid: pid.as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cmd,
                user,
                cpu: p.cpu_usage(),
                mem: p.memory(),
                mem_pct: (p.memory() as f64 / mem_total as f64 * 100.0) as f32,
                run_time: p.run_time(),
                status,
                threads,
            });
        }

        let load = System::load_average()
            .map(|l| (l.one, l.five, l.fifteen))
            .unwrap_or((0.0, 0.0, 0.0));

        let snap = Snapshot {
            cpu_total: sys.global_cpu_usage(),
            cpu_cores: sys.cpus().iter().map(|c| c.usage()).collect(),
            cpu_freq_mhz: sys.cpus().first().map(|c| c.frequency()).unwrap_or(0),
            mem_total,
            mem_used: sys.used_memory(),
            mem_available: sys.available_memory(),
            swap_total: sys.total_swap(),
            swap_used: sys.used_swap(),
            load,
            uptime: System::uptime().unwrap_or(0),
            rx_rate,
            tx_rate,
            rx_total,
            tx_total,
            ifaces,
            disks: disk_list,
            procs,
            n_running,
            n_threads,
            sample_ms: started.elapsed().as_millis() as u64,
        };

        if out.send(snap).is_err() {
            return;
        }

        // Sleep until the next wall-clock multiple of the interval, so that
        // several instances with the same interval refresh in lockstep. If
        // that boundary is too close for a meaningful CPU delta, skip to the
        // one after it.
        let mut wait = until_next_tick(interval);
        if wait < MINIMUM_CPU_UPDATE_INTERVAL {
            wait += interval;
        }
        thread::sleep(wait);
    }
}

/// Time left until the wall clock reaches the next multiple of `interval`
/// (counted from the Unix epoch). Always in `(0, interval]`.
pub fn until_next_tick(interval: Duration) -> Duration {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let step = interval.as_nanos().max(1);
    Duration::from_nanos((step - now % step) as u64)
}

fn collect_disks(disks: &Disks) -> Vec<DiskInfo> {
    let mut out: Vec<DiskInfo> = Vec::new();
    for d in disks {
        let total = d.total_space();
        if total == 0 {
            continue;
        }
        let mount = d.mount_point().to_string_lossy().into_owned();
        // Hide macOS internal volumes and Linux snap/loop mounts that only add noise.
        if mount.starts_with("/System/Volumes/")
            || mount.starts_with("/private/var/vm")
            || mount.contains("/snap/")
            || mount.starts_with("/boot/efi")
        {
            continue;
        }
        let name = d.name().to_string_lossy().into_owned();
        // Deduplicate APFS volumes sharing one container (same total + same device).
        if out.iter().any(|o| o.name == name && o.total == total) {
            continue;
        }
        out.push(DiskInfo {
            name,
            mount,
            total,
            used: total.saturating_sub(d.available_space()),
        });
    }
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

#[cfg(test)]
impl Sampler {
    /// Build a sampler with no background thread. Snapshots pushed into the
    /// returned sender show up in `latest()`, and commands sent via `send()`
    /// can be read back from the returned receiver.
    pub fn fake(info: StaticInfo) -> (Self, Sender<Snapshot>, Receiver<Command>) {
        let (snap_tx, snap_rx) = std::sync::mpsc::channel();
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let sampler = Self {
            info,
            rx: snap_rx,
            tx: cmd_tx,
        };
        (sampler, snap_tx, cmd_rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn until_next_tick_is_within_one_interval() {
        for ms in [250, 1000, 2000, 1750] {
            let iv = Duration::from_millis(ms);
            let wait = until_next_tick(iv);
            assert!(wait > Duration::ZERO && wait <= iv, "{wait:?} for {iv:?}");
        }
    }

    #[test]
    fn latest_returns_newest_and_drains_queue() {
        let (sampler, tx, _cmds) = Sampler::fake(StaticInfo::default());
        assert!(sampler.latest().is_none());

        for uptime in 1..=3 {
            tx.send(Snapshot {
                uptime,
                ..Default::default()
            })
            .unwrap();
        }
        assert_eq!(sampler.latest().unwrap().uptime, 3);
        assert!(sampler.latest().is_none());
    }

    #[test]
    fn send_forwards_commands() {
        let (sampler, _tx, cmds) = Sampler::fake(StaticInfo::default());
        sampler.send(Command::SetInterval(Duration::from_millis(750)));
        sampler.send(Command::Stop);
        assert!(matches!(
            cmds.try_recv(),
            Ok(Command::SetInterval(d)) if d == Duration::from_millis(750)
        ));
        assert!(matches!(cmds.try_recv(), Ok(Command::Stop)));
        assert!(cmds.try_recv().is_err());
    }
}

use std::time::Instant;

pub const MONITOR_SCRIPT: &str = "while true; do echo @@OXIDAL@@; echo @sys; uname -n 2>/dev/null; echo @user; whoami 2>/dev/null; echo @stat; cat /proc/stat 2>/dev/null; echo @mem; cat /proc/meminfo 2>/dev/null; echo @net; cat /proc/net/dev 2>/dev/null; echo @df; df -kP 2>/dev/null; sleep 2; done";

const MARKER: &[u8] = b"@@OXIDAL@@";

/// One mounted filesystem reported by `df`.
#[derive(Clone)]
pub struct DiskInfo {
    pub filesystem: String,
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Default)]
pub struct RemoteStats {
    pub sysname: Option<String>,
    pub user: Option<String>,
    pub cpu: Option<f32>,
    pub mem: Option<(u64, u64)>,
    /// The root filesystem (or the first real one when `/` isn't listed),
    /// shown in the monitor bar; `disks` has the full breakdown.
    pub disk: Option<(u64, u64)>,
    pub net: Option<(f64, f64)>,
    pub disks: Vec<DiskInfo>,
}

#[derive(Default)]
pub struct FrameSplitter {
    buf: Vec<u8>,
}

impl FrameSplitter {
    pub fn push(&mut self, data: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(data);

        let mut starts = Vec::new();
        let mut i = 0;
        while i + MARKER.len() <= self.buf.len() {
            if &self.buf[i..i + MARKER.len()] == MARKER {
                starts.push(i);
                i += MARKER.len();
            } else {
                i += 1;
            }
        }
        if starts.len() < 2 {
            return Vec::new();
        }

        let frames = starts
            .windows(2)
            .map(|w| String::from_utf8_lossy(&self.buf[w[0] + MARKER.len()..w[1]]).into_owned())
            .collect();
        self.buf.drain(..*starts.last().unwrap());
        frames
    }
}

#[derive(Default)]
pub struct StatsParser {
    prev_cpu: Option<(u64, u64)>,
    prev_net: Option<(u64, u64, Instant)>,
}

impl StatsParser {
    pub fn parse_frame(&mut self, frame: &str) -> RemoteStats {
        let mut stats = RemoteStats::default();
        let mut section = "";
        let mut cpu_sample = None;
        let mut mem_total = None;
        let mut mem_available = None;
        let mut net_sample: Option<(u64, u64)> = None;

        for line in frame.lines() {
            let line = line.trim_end();
            if let Some(name) = line.strip_prefix('@') {
                section = name;
                continue;
            }
            match section {
                "sys" => {
                    if stats.sysname.is_none() && !line.trim().is_empty() {
                        stats.sysname = Some(line.trim().to_string());
                    }
                }
                "user" => {
                    if stats.user.is_none() && !line.trim().is_empty() {
                        stats.user = Some(line.trim().to_string());
                    }
                }
                "stat" => {
                    if let Some(rest) = line.strip_prefix("cpu ") {
                        let fields: Vec<u64> = rest
                            .split_whitespace()
                            .filter_map(|f| f.parse().ok())
                            .collect();
                        if fields.len() >= 5 {
                            let total: u64 = fields.iter().take(8).sum();
                            let idle = fields[3] + fields[4];
                            cpu_sample = Some((idle, total));
                        }
                    }
                }
                "mem" => {
                    let mut parts = line.split_whitespace();
                    match (parts.next(), parts.next().and_then(|v| v.parse::<u64>().ok())) {
                        (Some("MemTotal:"), Some(kb)) => mem_total = Some(kb * 1024),
                        (Some("MemAvailable:"), Some(kb)) => mem_available = Some(kb * 1024),
                        _ => {}
                    }
                }
                "net" => {
                    if let Some((iface, rest)) = line.split_once(':') {
                        if iface.trim() != "lo" {
                            let fields: Vec<u64> = rest
                                .split_whitespace()
                                .filter_map(|f| f.parse().ok())
                                .collect();
                            if fields.len() >= 9 {
                                let (rx, tx) = net_sample.unwrap_or((0, 0));
                                net_sample = Some((rx + fields[0], tx + fields[8]));
                            }
                        }
                    }
                }
                "df" => {
                    // POSIX format: Filesystem 1024-blocks Used Available
                    // Capacity Mounted-on. The header row fails the numeric
                    // parses and is skipped naturally.
                    let fields: Vec<&str> = line.split_whitespace().collect();
                    if fields.len() >= 6
                        && let (Ok(total), Ok(used)) =
                            (fields[1].parse::<u64>(), fields[2].parse::<u64>())
                        && total > 0
                        && !is_pseudo_fs(fields[0])
                    {
                        stats.disks.push(DiskInfo {
                            filesystem: fields[0].to_string(),
                            mount: fields[5..].join(" "),
                            used: used * 1024,
                            total: total * 1024,
                        });
                    }
                }
                _ => {}
            }
        }

        stats.disks.sort_by(|a, b| a.mount.cmp(&b.mount));
        if let Some(disk) = stats
            .disks
            .iter()
            .find(|d| d.mount == "/")
            .or_else(|| stats.disks.first())
        {
            stats.disk = Some((disk.used, disk.total));
        }

        if let (Some(total), Some(available)) = (mem_total, mem_available) {
            stats.mem = Some((total.saturating_sub(available), total));
        }

        if let Some((idle, total)) = cpu_sample {
            if let Some((prev_idle, prev_total)) = self.prev_cpu {
                let dt = total.saturating_sub(prev_total);
                let di = idle.saturating_sub(prev_idle);
                if dt > 0 {
                    stats.cpu = Some((dt.saturating_sub(di) as f32 / dt as f32).clamp(0., 1.));
                }
            }
            self.prev_cpu = Some((idle, total));
        }

        if let Some((rx, tx)) = net_sample {
            let now = Instant::now();
            if let Some((prev_rx, prev_tx, prev_time)) = self.prev_net {
                let elapsed = now.duration_since(prev_time).as_secs_f64();
                if elapsed > 0.1 {
                    stats.net = Some((
                        rx.saturating_sub(prev_rx) as f64 / elapsed,
                        tx.saturating_sub(prev_tx) as f64 / elapsed,
                    ));
                }
            }
            self.prev_net = Some((rx, tx, now));
        }

        stats
    }
}

/// Virtual filesystems that would clutter the disk breakdown.
fn is_pseudo_fs(filesystem: &str) -> bool {
    matches!(
        filesystem,
        "tmpfs"
            | "devtmpfs"
            | "udev"
            | "none"
            | "overlay"
            | "squashfs"
            | "efivarfs"
            | "devfs"
            | "map"
            | "shm"
            | "cgroup"
            | "proc"
            | "sysfs"
    )
}

use std::time::Instant;

pub const MONITOR_SCRIPT: &str = "echo @@OXIDAL@@; i=0; while true; do \
if [ $i -eq 0 ]; then echo @sys; uname -n 2>/dev/null; echo @kernel; uname -s 2>/dev/null; echo @os; cat /etc/os-release 2>/dev/null; echo @user; whoami 2>/dev/null; fi; \
if [ $((i % 5)) -eq 0 ]; then echo @who; who 2>/dev/null; echo @df; df -kP 2>/dev/null; echo @sensors; command -v sensors 2>/dev/null; echo @temp; sensors -u 2>/dev/null; fi; \
echo @uptime; { read -r u rest < /proc/uptime && printf '%s\\n' \"$u\"; } 2>/dev/null || { sysctl -n kern.boottime; date +%s; } 2>/dev/null; \
echo @stat; { read -r l < /proc/stat && printf '%s\\n' \"$l\"; } 2>/dev/null; \
echo @mem; { while read -r k v rest; do case $k in MemTotal:|MemAvailable:) printf '%s\\n' \"$k $v\";; esac; done < /proc/meminfo; } 2>/dev/null; \
echo @net; { while read -r l; do printf '%s\\n' \"$l\"; done < /proc/net/dev; } 2>/dev/null; \
echo @conn; command -v netstat >/dev/null 2>&1 && { echo netstat; netstat -tn 2>/dev/null | grep ESTABLISHED; }; \
echo @@OXIDAL@@; i=$((i + 1)); sleep 1; done";

const MARKER: &[u8] = b"@@OXIDAL@@";
const CPU_CHIPS: &[&str] = &[
    "coretemp",
    "k10temp",
    "zenpower",
    "cpu_thermal",
    "cpu-thermal",
];
const DEFAULT_TEMP_LIMIT: f32 = 100.;

#[derive(Clone)]
pub struct DiskInfo {
    pub filesystem: String,
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TempInfo {
    pub chip: String,
    pub label: String,
    pub value: f32,
    pub limit: Option<f32>,
}

impl TempInfo {
    pub fn fraction(&self) -> f32 {
        (self.value / self.limit.unwrap_or(DEFAULT_TEMP_LIMIT)).clamp(0., 1.)
    }

    fn is_cpu(&self) -> bool {
        CPU_CHIPS.iter().any(|chip| self.chip.starts_with(chip))
    }
}

pub fn headline_temp(temps: &[TempInfo]) -> Option<&TempInfo> {
    let hottest = |pred: fn(&TempInfo) -> bool| {
        temps
            .iter()
            .filter(|t| pred(t))
            .max_by(|a, b| a.value.total_cmp(&b.value))
    };
    hottest(TempInfo::is_cpu).or_else(|| hottest(|_| true))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OsLogo {
    pub icon: &'static str,
    pub color: u32,
}

const OS_LOGOS: &[(&[&str], OsLogo)] = &[
    (&["ubuntu"], logo("ubuntu", 0xE95420)),
    (&["debian"], logo("debian", 0xA81D33)),
    (&["raspbian"], logo("raspberrypi", 0xA22846)),
    (&["alpine"], logo("alpinelinux", 0x0D597F)),
    (&["arch", "archarm"], logo("archlinux", 0x1793D1)),
    (&["manjaro", "manjaro-arm"], logo("manjaro", 0x35BF5C)),
    (&["endeavouros"], logo("endeavouros", 0x7F7FFF)),
    (&["fedora"], logo("fedora", 0x51A2DA)),
    (&["centos"], logo("centos", 0x262577)),
    (&["rhel"], logo("redhat", 0xEE0000)),
    (&["rocky"], logo("rockylinux", 0x10B981)),
    (&["almalinux"], logo("almalinux", 0x000000)),
    (
        &[
            "opensuse",
            "opensuse-leap",
            "opensuse-tumbleweed",
            "opensuse-microos",
            "suse",
            "sles",
        ],
        logo("opensuse", 0x73BA25),
    ),
    (&["gentoo"], logo("gentoo", 0x54487A)),
    (&["linuxmint"], logo("linuxmint", 0x86BE43)),
    (&["nixos"], logo("nixos", 0x5277C3)),
    (&["kali"], logo("kalilinux", 0x557C94)),
    (&["pop"], logo("popos", 0x48B9C7)),
    (&["void"], logo("voidlinux", 0x478061)),
    (&["elementary"], logo("elementary", 0x64BAFF)),
    (&["freebsd"], logo("freebsd", 0xAB2B28)),
    (&["openbsd"], logo("openbsd", 0xF2CA30)),
    (&["netbsd"], logo("netbsd", 0xFF6600)),
    (&["darwin", "macos"], logo("apple", 0x000000)),
];
const GENERIC_LINUX: OsLogo = logo("linux", 0xFCC624);

const fn logo(icon: &'static str, color: u32) -> OsLogo {
    OsLogo { icon, color }
}

fn find_logo(id: &str) -> Option<OsLogo> {
    let id = id.to_ascii_lowercase();
    OS_LOGOS
        .iter()
        .find(|(ids, _)| ids.contains(&id.as_str()))
        .map(|(_, logo)| *logo)
}

fn resolve_os(id: Option<&str>, id_like: Option<&str>, kernel: Option<&str>) -> Option<OsLogo> {
    id.into_iter()
        .chain(id_like.into_iter().flat_map(str::split_whitespace))
        .chain(kernel)
        .find_map(find_logo)
        .or_else(|| (id.is_some() || kernel == Some("Linux")).then_some(GENERIC_LINUX))
}

fn os_release_value(line: &str, key: &str) -> Option<String> {
    let value = line.strip_prefix(key)?.strip_prefix('=')?.trim();
    let value = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
        .unwrap_or(value);
    (!value.is_empty()).then(|| value.to_string())
}

fn parse_boottime(line: &str) -> Option<u64> {
    let rest = &line[line.find("sec =")? + "sec =".len()..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[derive(Clone, Default)]
pub struct RemoteStats {
    pub sysname: Option<String>,
    pub os: Option<OsLogo>,
    pub uptime: Option<u64>,
    pub user: Option<String>,
    pub cpu: Option<f32>,
    pub mem: Option<(u64, u64)>,
    pub disk: Option<(u64, u64)>,
    pub net: Option<(f64, f64)>,
    pub disks: Vec<DiskInfo>,
    pub who: Vec<String>,
    pub connections: Option<(u32, u16)>,
    pub temps: Option<Vec<TempInfo>>,
}

#[derive(Default)]
pub struct FrameSplitter {
    buf: Vec<u8>,
    scanned: usize,
    starts: Vec<usize>,
}

impl FrameSplitter {
    pub fn push(&mut self, data: &[u8]) -> Option<String> {
        self.buf.extend_from_slice(data);

        let mut i = self.scanned;
        while i + MARKER.len() <= self.buf.len() {
            if &self.buf[i..i + MARKER.len()] == MARKER {
                self.starts.push(i);
                i += MARKER.len();
            } else {
                i += 1;
            }
        }
        self.scanned = i;

        if self.starts.len() < 2 {
            return None;
        }

        let last = self.starts[self.starts.len() - 2];
        let cut = self.starts[self.starts.len() - 1];
        let frame = String::from_utf8_lossy(&self.buf[last + MARKER.len()..cut]).into_owned();
        self.buf.drain(..cut);
        self.scanned -= cut;
        self.starts.clear();
        self.starts.push(0);
        Some(frame)
    }
}

pub struct StatsParser {
    port: u16,
    prev_cpu: Option<(u64, u64)>,
    prev_net: Option<(u64, u64, Instant)>,
    last: RemoteStats,
}

#[derive(Default)]
struct Seen {
    sys: bool,
    os: bool,
    user: bool,
    who: bool,
    df: bool,
    sensors: bool,
}

impl StatsParser {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            prev_cpu: None,
            prev_net: None,
            last: RemoteStats::default(),
        }
    }

    pub fn parse_frame(&mut self, frame: &str) -> RemoteStats {
        let mut seen = Seen::default();
        let mut stats = self.parse_sections(frame, &mut seen);
        let last = &mut self.last;
        if seen.sys {
            last.sysname = stats.sysname.clone();
        } else {
            stats.sysname = last.sysname.clone();
        }
        if seen.os {
            last.os = stats.os;
        } else {
            stats.os = last.os;
        }
        if seen.user {
            last.user = stats.user.clone();
        } else {
            stats.user = last.user.clone();
        }
        if seen.who {
            last.who = stats.who.clone();
        } else {
            stats.who = last.who.clone();
        }
        if seen.df {
            last.disks = stats.disks.clone();
            last.disk = stats.disk;
        } else {
            stats.disks = last.disks.clone();
            stats.disk = last.disk;
        }
        if seen.sensors {
            last.temps = stats.temps.clone();
        } else {
            stats.temps = last.temps.clone();
        }
        stats
    }

    fn parse_sections(&mut self, frame: &str, seen: &mut Seen) -> RemoteStats {
        let mut stats = RemoteStats::default();
        let mut section = "";
        let mut cpu_sample = None;
        let mut mem_total = None;
        let mut mem_available = None;
        let mut net_sample: Option<(u64, u64)> = None;
        let mut conn_output = false;
        let mut conn_count = 0u32;
        let mut sensors_found = false;
        let mut temp_lines: Vec<&str> = Vec::new();
        let mut kernel = None;
        let mut os_id = None;
        let mut os_like = None;
        let mut boot_time = None;
        let port_str = self.port.to_string();

        for line in frame.lines() {
            let line = line.trim_end();
            if let Some(name) = line.strip_prefix('@') {
                section = name;
                match name {
                    "sys" => seen.sys = true,
                    "os" | "kernel" => seen.os = true,
                    "user" => seen.user = true,
                    "who" => seen.who = true,
                    "df" => seen.df = true,
                    "sensors" => seen.sensors = true,
                    _ => {}
                }
                continue;
            }
            match section {
                "sys" => {
                    if stats.sysname.is_none() && !line.trim().is_empty() {
                        stats.sysname = Some(line.trim().to_string());
                    }
                }
                "kernel" => {
                    if kernel.is_none() && !line.trim().is_empty() {
                        kernel = Some(line.trim().to_string());
                    }
                }
                "os" => {
                    if let Some(id) = os_release_value(line, "ID") {
                        os_id = Some(id);
                    } else if let Some(like) = os_release_value(line, "ID_LIKE") {
                        os_like = Some(like);
                    }
                }
                "uptime" => {
                    if let Some(boot) = parse_boottime(line) {
                        boot_time = Some(boot);
                    } else if stats.uptime.is_none()
                        && let Some(value) = line
                            .split_whitespace()
                            .next()
                            .and_then(|v| v.parse::<f64>().ok())
                    {
                        stats.uptime = Some(match boot_time {
                            Some(boot) => (value as u64).saturating_sub(boot),
                            None => value as u64,
                        });
                    }
                }
                "user" => {
                    if stats.user.is_none() && !line.trim().is_empty() {
                        stats.user = Some(line.trim().to_string());
                    }
                }
                "who" => {
                    if !line.trim().is_empty() {
                        stats.who.push(line.trim().to_string());
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
                    match (
                        parts.next(),
                        parts.next().and_then(|v| v.parse::<u64>().ok()),
                    ) {
                        (Some("MemTotal:"), Some(kb)) => mem_total = Some(kb * 1024),
                        (Some("MemAvailable:"), Some(kb)) => mem_available = Some(kb * 1024),
                        _ => {}
                    }
                }
                "conn" => {
                    if !line.trim().is_empty() {
                        conn_output = true;
                        let fields: Vec<&str> = line.split_whitespace().collect();
                        if line.starts_with("tcp")
                            && fields.len() >= 6
                            && fields[5] == "ESTABLISHED"
                            && fields[3].rsplit(':').next() == Some(port_str.as_str())
                        {
                            conn_count += 1;
                        }
                    }
                }
                "net" => {
                    if let Some((iface, rest)) = line.split_once(':')
                        && iface.trim() != "lo"
                    {
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
                "sensors" => {
                    if !line.trim().is_empty() {
                        sensors_found = true;
                    }
                }
                "temp" => temp_lines.push(line),
                "df" => {
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

        stats.os = resolve_os(os_id.as_deref(), os_like.as_deref(), kernel.as_deref());

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

        if conn_output {
            stats.connections = Some((conn_count, self.port));
        }

        if sensors_found {
            stats.temps = Some(parse_sensors(&temp_lines));
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

#[derive(Default)]
struct TempReading {
    chip: String,
    label: String,
    value: Option<f32>,
    high: Option<f32>,
    crit: Option<f32>,
}

impl TempReading {
    fn flush(&mut self, out: &mut Vec<TempInfo>) {
        let (high, crit) = (self.high.take(), self.crit.take());
        if let Some(value) = self.value.take() {
            out.push(TempInfo {
                chip: self.chip.clone(),
                label: std::mem::take(&mut self.label),
                value,
                limit: high.or(crit),
            });
        }
    }
}

fn sane_limit(value: f32) -> Option<f32> {
    (value > 0. && value < 200.).then_some(value)
}

fn parse_sensors(lines: &[&str]) -> Vec<TempInfo> {
    let mut temps = Vec::new();
    let mut reading = TempReading::default();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            reading.flush(&mut temps);
            reading.chip.clear();
        } else if let Some((key, value)) = trimmed.split_once(':')
            && let Ok(value) = value.trim().parse::<f32>()
        {
            let kind = key
                .strip_prefix("temp")
                .and_then(|k| k.split_once('_'))
                .map(|(_, kind)| kind);
            match kind {
                Some("input") => reading.value = Some(value),
                Some("max") => reading.high = sane_limit(value),
                Some("crit") => reading.crit = sane_limit(value),
                _ => {}
            }
        } else if trimmed.starts_with("Adapter:") {
        } else if let Some(label) = trimmed.strip_suffix(':') {
            reading.flush(&mut temps);
            reading.label = label.to_string();
        } else {
            reading.flush(&mut temps);
            reading.chip = trimmed.to_string();
        }
    }
    reading.flush(&mut temps);
    temps
}

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

#[cfg(test)]
#[path = "../../tests/terminal/stats.rs"]
mod tests;

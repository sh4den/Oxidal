use super::*;
use crate::session::SessionKind;

#[test]
fn filter_matches_name_host_user_and_kind() {
    let mut s = Session::new("prod-web", SessionKind::Ssh);
    s.host = "10.0.1.12".into();
    s.username = "deploy".into();

    for query in ["prod", "0.1.1", "deploy", "ssh"] {
        assert!(matches_filter(&s, query), "{query}");
    }
    assert!(!matches_filter(&s, "sftp"));
    assert!(matches_filter(&s, ""));
}

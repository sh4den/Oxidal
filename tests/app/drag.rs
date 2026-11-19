use super::*;
use crate::session::SessionKind;

fn session(name: &str, folder_id: Option<Uuid>) -> Session {
    let mut session = Session::new(name, SessionKind::Ssh);
    session.folder_id = folder_id;
    session
}

fn names(sessions: &[Session]) -> Vec<&str> {
    sessions.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn dropping_before_a_row_reorders_and_adopts_its_folder() {
    let folder = Uuid::new_v4();
    let mut sessions = vec![
        session("a", None),
        session("b", None),
        session("c", Some(folder)),
    ];
    let (a, c) = (sessions[0].id, sessions[2].id);

    assert!(reorder_sessions(&mut sessions, a, DropTarget::Before(c)));
    assert_eq!(names(&sessions), vec!["b", "a", "c"]);
    assert_eq!(sessions[1].folder_id, Some(folder));
}

#[test]
fn dropping_into_a_folder_appends_and_ungrouping_clears_the_folder() {
    let folder = Uuid::new_v4();
    let mut sessions = vec![
        session("a", None),
        session("b", Some(folder)),
        session("c", Some(folder)),
    ];
    let (a, b) = (sessions[0].id, sessions[1].id);

    assert!(reorder_sessions(
        &mut sessions,
        a,
        DropTarget::Into(Some(folder))
    ));
    assert_eq!(names(&sessions), vec!["b", "c", "a"]);
    assert_eq!(sessions[2].folder_id, Some(folder));

    assert!(reorder_sessions(&mut sessions, b, DropTarget::Into(None)));
    assert_eq!(names(&sessions), vec!["c", "a", "b"]);
    assert_eq!(sessions[2].folder_id, None);
}

#[test]
fn unknown_targets_leave_the_list_untouched() {
    let mut sessions = vec![session("a", None), session("b", None)];
    let b = sessions[1].id;

    assert!(!reorder_sessions(
        &mut sessions,
        b,
        DropTarget::Before(Uuid::new_v4())
    ));
    assert_eq!(names(&sessions), vec!["a", "b"]);
    assert!(!reorder_sessions(
        &mut sessions,
        Uuid::new_v4(),
        DropTarget::Into(None)
    ));
    assert_eq!(names(&sessions), vec!["a", "b"]);
}

#[test]
fn folders_reorder_before_a_target_or_to_the_end() {
    let mut folders = vec![
        SessionFolder::new("a"),
        SessionFolder::new("b"),
        SessionFolder::new("c"),
    ];
    let (a, c) = (folders[0].id, folders[2].id);
    let folder_names =
        |folders: &[SessionFolder]| folders.iter().map(|f| f.name.clone()).collect::<Vec<_>>();

    assert!(reorder_folders(&mut folders, c, Some(a)));
    assert_eq!(folder_names(&folders), ["c", "a", "b"]);
    assert!(reorder_folders(&mut folders, c, None));
    assert_eq!(folder_names(&folders), ["a", "b", "c"]);
    assert!(!reorder_folders(&mut folders, a, Some(Uuid::new_v4())));
    assert_eq!(folder_names(&folders), ["a", "b", "c"]);
}

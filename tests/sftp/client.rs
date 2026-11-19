use super::*;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

#[test]
fn planning_an_upload_walks_the_whole_tree() {
    let base = std::env::temp_dir().join(format!("oxidal-plan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("nested").join("deeper")).expect("dirs");
    std::fs::write(base.join("top.txt"), b"12345").expect("file");
    std::fs::write(base.join("nested").join("mid.txt"), b"12").expect("file");
    std::fs::write(base.join("nested").join("deeper").join("leaf.bin"), b"123").expect("file");

    let (mut dirs, files) = block_on(plan_upload(&base, "/srv/dest")).expect("plan");
    dirs.sort();

    assert_eq!(
        dirs,
        vec![
            "/srv/dest".to_string(),
            "/srv/dest/nested".to_string(),
            "/srv/dest/nested/deeper".to_string(),
        ],
        "every directory should be created remotely, parents before children once sorted"
    );

    let mut destinations: Vec<&str> = files.iter().map(|file| file.destination.as_str()).collect();
    destinations.sort();
    assert_eq!(
        destinations,
        vec![
            "/srv/dest/nested/deeper/leaf.bin",
            "/srv/dest/nested/mid.txt",
            "/srv/dest/top.txt",
        ],
        "remote destinations stay slash separated whatever the local separator is"
    );

    assert_eq!(
        files.iter().map(|file| file.size).sum::<u64>(),
        10,
        "the progress total should cover every file in the tree"
    );

    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn reconnect_backoff_doubles_up_to_a_ceiling() {
    let delays: Vec<u64> = (0..RECONNECT_ATTEMPTS)
        .map(|attempt| reconnect_delay(attempt).as_secs())
        .collect();
    assert_eq!(delays, vec![1, 2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(reconnect_delay(u32::MAX), RECONNECT_MAX_DELAY);
}

#[test]
fn planning_an_upload_of_an_empty_directory_still_creates_it() {
    let base = std::env::temp_dir().join(format!("oxidal-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).expect("dir");

    let (dirs, files) = block_on(plan_upload(&base, "/srv/empty")).expect("plan");

    assert_eq!(dirs, vec!["/srv/empty".to_string()]);
    assert!(files.is_empty());

    let _ = std::fs::remove_dir_all(&base);
}

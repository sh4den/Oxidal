use super::*;

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::NaiveDate::from_ymd_opt(2026, 3, 10)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
}

fn timestamp(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Option<u64> {
    Some(
        chrono::NaiveDate::from_ymd_opt(y, mo, d)
            .unwrap()
            .and_hms_opt(h, mi, 0)
            .unwrap()
            .and_utc()
            .timestamp() as u64,
    )
}

#[test]
fn multiline_replies_and_codes() {
    assert_eq!(parse_code("230 Login successful."), Some(230));
    assert_eq!(parse_code("211-Features:"), Some(211));
    assert_eq!(parse_code("220"), Some(220));
    assert_eq!(parse_code("2200 nope"), None);
    assert_eq!(parse_code(" 211 End"), None);

    let reply = Reply {
        code: 211,
        text: "211-Features:\n MDTM\n MLST type*;size*;modify*;\n UTF8\n SIZE\n EPSV\n211 End"
            .to_string(),
    };
    let features = parse_features(&reply);
    assert!(features.mlsd && features.utf8 && features.size);
}

#[test]
fn reply_messages_keep_the_code() {
    let reply = Reply {
        code: 550,
        text: "550 Permission denied.".to_string(),
    };
    assert_eq!(reply.message(), "Permission denied. (550)");
    assert!(reply.completed().is_err());
}

#[test]
fn passive_replies() {
    assert_eq!(
        parse_pasv("227 Entering Passive Mode (192,168,1,10,195,149)."),
        Some(195 * 256 + 149)
    );
    assert_eq!(
        parse_pasv("227 Entering Passive Mode 10,0,0,1,4,1"),
        Some(1025)
    );
    assert_eq!(parse_pasv("227 Entering Passive Mode (1,2,3)"), None);
    assert_eq!(
        parse_epsv("229 Entering Extended Passive Mode (|||50123|)"),
        Some(50123)
    );
    assert_eq!(parse_epsv("229 EPSV ok (!!!6446!)"), Some(6446));
}

#[test]
fn pwd_handles_quotes_inside_the_path() {
    assert_eq!(
        parse_pwd("257 \"/home/ftp\" is the current directory"),
        Some("/home/ftp".to_string())
    );
    assert_eq!(
        parse_pwd("257 \"/odd \"\"name\"\"\" here"),
        Some("/odd \"name\"".to_string())
    );
    assert_eq!(parse_pwd("257 no quotes"), None);
}

#[test]
fn mlsd_lines() {
    let entry = parse_mlsd_line(
            "type=file;size=1024;modify=20250102030405.123;UNIX.mode=0644;UNIX.owner=web;UNIX.group=www; index.html",
        )
        .expect("file");
    assert_eq!(entry.name, "index.html");
    assert!(!entry.is_dir);
    assert_eq!(entry.size, 1024);
    assert_eq!(entry.permissions, Some(0o644));
    assert_eq!(entry.owner.as_deref(), Some("web"));
    assert_eq!(entry.group.as_deref(), Some("www"));
    assert_eq!(entry.modified, timestamp(2025, 1, 2, 3, 4).map(|t| t + 5));

    let dir = parse_mlsd_line("type=dir;modify=20250102030405; my folder").expect("dir");
    assert!(dir.is_dir);
    assert_eq!(dir.name, "my folder", "spaces in names survive");

    let link = parse_mlsd_line("type=OS.unix=slink:/etc;size=4; etc-link").expect("link");
    assert!(link.is_symlink);

    assert!(parse_mlsd_line("type=cdir;modify=20250102030405; .").is_none());
    assert!(parse_mlsd_line("type=pdir;modify=20250102030405; ..").is_none());
}

#[test]
fn unix_list_lines() {
    let file = parse_list_line(
        "-rw-r--r--    1 1000     1000         5120 Jan 05 14:30 notes with  spaces.txt",
        now(),
    )
    .expect("file");
    assert_eq!(file.name, "notes with  spaces.txt");
    assert_eq!(file.size, 5120);
    assert_eq!(file.permissions, Some(0o644));
    assert_eq!(file.owner.as_deref(), Some("1000"));
    assert_eq!(file.modified, timestamp(2026, 1, 5, 14, 30));

    let last_year = parse_list_line(
        "drwxr-xr-x    2 ftp      ftp          4096 Dec 24 09:00 archive",
        now(),
    )
    .expect("dir");
    assert!(last_year.is_dir);
    assert_eq!(
        last_year.modified,
        timestamp(2025, 12, 24, 9, 0),
        "a date later than today belongs to last year"
    );

    let old = parse_list_line("-rwxr-sr-x 1 root 123456 Mar  1  2019 setup.sh", now())
        .expect("no group column");
    assert_eq!(old.size, 123456);
    assert_eq!(old.owner.as_deref(), Some("root"));
    assert_eq!(old.group, None);
    assert_eq!(old.permissions, Some(0o755));
    assert_eq!(old.modified, timestamp(2019, 3, 1, 0, 0));

    let link = parse_list_line(
        "lrwxrwxrwx    1 0        0               7 Feb 01 08:00 www -> /var/www",
        now(),
    )
    .expect("link");
    assert!(link.is_symlink);
    assert_eq!(link.name, "www");

    assert!(parse_list_line("total 24", now()).is_none());
}

#[test]
fn dos_list_lines() {
    let dir = parse_list_line(
        "01-02-23  03:04PM       <DIR>          Program Files",
        now(),
    )
    .expect("dir");
    assert!(dir.is_dir);
    assert_eq!(dir.name, "Program Files");
    assert_eq!(dir.modified, timestamp(2023, 1, 2, 15, 4));

    let file =
        parse_list_line("11-30-2024  12:15AM              1,234 readme.txt", now()).expect("file");
    assert_eq!(file.size, 1234);
    assert_eq!(file.modified, timestamp(2024, 11, 30, 0, 15));
}

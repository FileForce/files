use super::*;

fn repo(name: &str) -> PathBuf {
    files_sdk::permissions::declare(&MANIFEST);
    let dir = std::env::temp_dir().join(format!("files-ext-git-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    init(&dir).unwrap();
    for (key, value) in [("user.name", "Tester"), ("user.email", "t@example.com"), ("commit.gpgsign", "false")] {
        run(&dir, &["config", key, value]).unwrap();
    }
    dir
}

#[test]
fn status_commit_log_diff_and_revert() {
    let dir = repo("flow");
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/b.rs"), "fn b() {}\n").unwrap();
    assert_eq!(repo_root(&dir.join("src")).map(|root| root.canonicalize().unwrap()), Some(dir.canonicalize().unwrap()));
    assert!(log(&dir, None, 10).unwrap().is_empty(), "no commits yet is not an error");

    let status = status(&dir).unwrap();
    assert_eq!(status.changes.iter().map(|change| (change.path.as_str(), change.kind)).collect::<Vec<_>>(),
        [("a.txt", ChangeKind::Untracked), ("src/b.rs", ChangeKind::Untracked)]);

    // Commit only what was ticked.
    let made = commit(&dir, &["a.txt".to_string()], "Add a").unwrap();
    assert!(made.ends_with("Add a"), "{made}");
    let status = super::status(&dir).unwrap();
    assert_eq!(status.changes.len(), 1, "b.rs was not ticked");

    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    let diff = diff_worktree(&dir, "a.txt").unwrap();
    let lines = diff_lines(&diff);
    assert!(lines.contains(&(LineKind::Added, "+two".to_string())), "{diff}");
    assert!(diff_lines(&diff_worktree(&dir, "src/b.rs").unwrap()).iter().any(|(kind, line)| *kind == LineKind::Added && line == "+fn b() {}"));

    revert(&dir, &[dir.join("a.txt")]).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "one\n");

    let commits = log(&dir, None, 10).unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!((commits[0].author.as_str(), commits[0].subject.as_str()), ("Tester", "Add a"));
    assert!(commits[0].refs.iter().any(|name| name.contains("HEAD")));
    let files = commit_files(&dir, &commits[0].hash).unwrap();
    assert_eq!((files[0].path.as_str(), files[0].kind), ("a.txt", ChangeKind::Added));
    assert!(diff_commit(&dir, &commits[0].hash, "a.txt").unwrap().contains("+one"));
    assert_eq!(log(&dir, Some(&dir.join("src/b.rs")), 10).unwrap().len(), 0, "log of one file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn branches_stash_and_clone() {
    let dir = repo("branches");
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    commit(&dir, &["a.txt".into()], "first").unwrap();
    create_branch(&dir, "feature", true).unwrap();
    assert!(create_branch(&dir, "has space", false).is_err());
    let names: Vec<_> = branches(&dir).unwrap().into_iter().map(|branch| (branch.name, branch.current)).collect();
    assert!(names.contains(&("feature".to_string(), true)), "{names:?}");
    let main = names.iter().find(|(name, _)| name != "feature").unwrap().0.clone();
    switch(&dir, &main, false).unwrap();
    assert_eq!(status(&dir).unwrap().branch.as_deref(), Some(main.as_str()));

    std::fs::write(dir.join("a.txt"), "2\n").unwrap();
    stash_save(&dir, Some("wip")).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "1\n");
    assert_eq!(stash_count(&dir), 1);
    stash_pop(&dir).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "2\n");

    // Clone from a local path (no network needed).
    let parent = dir.with_extension("clones");
    let _ = std::fs::remove_dir_all(&parent);
    std::fs::create_dir_all(&parent).unwrap();
    let cloned = clone(&dir.display().to_string(), &parent).unwrap();
    assert!(cloned.join("a.txt").exists());
    assert!(clone(&dir.display().to_string(), &parent).is_err(), "never clones over a folder");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&parent);
}

#[test]
fn parses_porcelain_status() {
    let output = "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n\
        1 .M N... 100644 100644 100644 aaa bbb src/lib.rs\n\
        1 A. N... 000000 100644 100644 000 ccc new file.txt\n\
        2 R. N... 100644 100644 100644 ddd eee R100 moved.rs\told.rs\n\
        u UU N... 100644 100644 100644 100644 f g h both.txt\n\
        ? notes.md\n";
    let status = parse_status(output);
    assert_eq!((status.branch.as_deref(), status.upstream.as_deref(), status.ahead, status.behind), (Some("main"), Some("origin/main"), 2, 1));
    let changes: Vec<_> = status.changes.iter().map(|change| (change.path.as_str(), change.kind, change.staged)).collect();
    assert_eq!(changes, [
        ("src/lib.rs", ChangeKind::Modified, false),
        ("new file.txt", ChangeKind::Added, true),
        ("moved.rs", ChangeKind::Renamed, true),
        ("both.txt", ChangeKind::Conflicted, false),
        ("notes.md", ChangeKind::Untracked, false),
    ]);
}

#[test]
fn writes_need_the_permission() {
    let dir = repo("perm");
    let mut revoked = std::collections::BTreeMap::new();
    revoked.insert(MANIFEST.id.to_string(), std::collections::BTreeSet::from([Permission::WriteFiles]));
    files_sdk::permissions::set_revoked(revoked);
    let error = commit(&dir, &["x".into()], "m").unwrap_err();
    files_sdk::permissions::set_revoked(Default::default());
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let _ = std::fs::remove_dir_all(&dir);
}

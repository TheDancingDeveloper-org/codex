//! Fork (WI-1132): the `all` project-doc mode.

use std::fs;

use codex_exec_server::LOCAL_FS;
use codex_utils_path_uri::PathUri;

use super::discover_compat_docs;
use super::expand_project_docs;

fn uri(path: &std::path::Path) -> PathUri {
    PathUri::from_host_native_path(path).expect("path uri")
}

fn write(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent");
    }
    fs::write(path, contents).expect("write");
}

/// Every matching filename is kept, and `.claude/rules/*.md` follows in order.
#[tokio::test]
async fn all_mode_keeps_every_doc_and_sorted_rules() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("AGENTS.md"), "agents\n");
    write(&dir.path().join("CLAUDE.md"), "claude\n");
    write(&dir.path().join(".claude/rules/b.md"), "rule b\n");
    write(&dir.path().join(".claude/rules/a.md"), "rule a\n");
    write(&dir.path().join(".claude/rules/notes.txt"), "not a rule\n");
    let root = uri(dir.path());

    let found = discover_compat_docs(
        LOCAL_FS.as_ref(),
        &root,
        &["AGENTS.md", "CLAUDE.md"],
        &root,
        None,
    )
    .await
    .expect("discover");

    let names = found
        .iter()
        .map(|path| path.basename().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(names, ["AGENTS.md", "CLAUDE.md", "a.md", "b.md"]);
}

/// An import that leaves the project is not read, whether it escapes by an
/// absolute path, `~/`, a `..` climb or a symlink.
#[tokio::test]
async fn imports_that_escape_the_project_root_are_refused() {
    let project = tempfile::tempdir().expect("project");
    let outside = tempfile::tempdir().expect("outside");
    write(&outside.path().join("secret.md"), "TOPSECRET\n");
    std::os::unix::fs::symlink(outside.path().join("secret.md"), project.path().join("escape.md"))
        .expect("symlink");
    let home = tempfile::tempdir().expect("home");
    write(&home.path().join("secret.md"), "TOPSECRET\n");
    // Safety: this test runs alone and only points HOME at a temp dir.
    unsafe { std::env::set_var("HOME", home.path()) };
    write(
        &project.path().join("CLAUDE.md"),
        "@escape.md\n@../secret.md\n@/etc/hostname\n@~/secret.md\n",
    );
    let root = uri(project.path());

    let docs = expand_project_docs(
        LOCAL_FS.as_ref(),
        &[root.join("CLAUDE.md").expect("join")],
        &root,
        4096,
        None,
    )
    .await
    .expect("expand");

    let text = docs
        .iter()
        .map(|doc| doc.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !text.contains("TOPSECRET"),
        "an import outside the project was read: {text}"
    );
    let read = docs.iter().map(|doc| doc.text.len()).sum::<usize>();
    let own = fs::read(project.path().join("CLAUDE.md")).unwrap().len();
    assert!(read <= own, "read {read} bytes, the project file is only {own}");
}

/// A relative import that stays inside the project is expanded.
#[tokio::test]
async fn imports_inside_the_project_root_are_expanded() {
    let project = tempfile::tempdir().expect("project");
    write(&project.path().join("CLAUDE.md"), "head\n@docs/more.md\n");
    write(&project.path().join("docs/more.md"), "body\n");
    let root = uri(project.path());

    let docs = expand_project_docs(
        LOCAL_FS.as_ref(),
        &[root.join("CLAUDE.md").expect("join")],
        &root,
        4096,
        None,
    )
    .await
    .expect("expand");

    let text = docs
        .iter()
        .map(|doc| doc.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains("body"), "the in-project import was not read: {text}");
}

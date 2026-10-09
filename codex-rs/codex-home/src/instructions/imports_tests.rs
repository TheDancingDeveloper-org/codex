use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use pretty_assertions::assert_eq;

use super::MAX_IMPORT_DEPTH;
use super::expand_imports;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, text).expect("write file");
}

#[tokio::test]
async fn line_only_import_is_inlined_and_counted() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("AGENTS.md"), "shared rules\n");
    write(
        &dir.path().join("CLAUDE.md"),
        "preamble\n@AGENTS.md\nclosing\n",
    );

    let expansion = expand_imports(&dir.path().join("CLAUDE.md"), None, 1024, None).await;

    assert_eq!(expansion.warnings, Vec::<String>::new());
    assert_eq!(expansion.files.len(), 1);
    assert_eq!(expansion.files[0].text, "preamble\nshared rules\nclosing");
    assert_eq!(
        expansion.bytes_read,
        fs::read(dir.path().join("CLAUDE.md")).unwrap().len()
            + fs::read(dir.path().join("AGENTS.md")).unwrap().len()
    );
}

#[tokio::test]
async fn imports_inside_fences_and_mid_line_are_left_alone() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("real.md"), "imported\n");
    write(
        &dir.path().join("CLAUDE.md"),
        "see @real.md inline\n```\n@real.md\n```\n~~~@real.md\n",
    );

    let expansion = expand_imports(&dir.path().join("CLAUDE.md"), None, 1024, None).await;

    assert_eq!(expansion.files.len(), 1);
    assert_eq!(
        expansion.files[0].text,
        "see @real.md inline\n```\n@real.md\n```\n~~~@real.md"
    );
    assert_eq!(
        expansion.bytes_read,
        fs::read(dir.path().join("CLAUDE.md")).unwrap().len()
    );
}

#[tokio::test]
async fn expansion_stops_at_depth_five() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("level6.md"), "too deep\n");
    let mut next = "level6.md".to_string();
    for level in (1..=MAX_IMPORT_DEPTH).rev() {
        let name = format!("level{level}.md");
        write(&dir.path().join(&name), &format!("@{next}\n"));
        next = name;
    }
    write(&dir.path().join("CLAUDE.md"), &format!("@{next}\n"));

    let expansion = expand_imports(&dir.path().join("CLAUDE.md"), None, 1024, None).await;

    let text = &expansion.files[0].text;
    assert!(
        text.contains("@level6.md"),
        "the import past depth {MAX_IMPORT_DEPTH} must stay literal, got {text:?}"
    );
    assert!(!text.contains("too deep"));
}

#[tokio::test]
async fn duplicate_path_symlink_and_body_are_kept_once() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("shared.md"), "same body\n");
    symlink("shared.md", dir.path().join("link.md")).expect("symlink");
    write(&dir.path().join("copy.md"), "same body\n");
    write(
        &dir.path().join("CLAUDE.md"),
        "@shared.md\n@link.md\n@copy.md\n@shared.md\n",
    );

    let expansion = expand_imports(&dir.path().join("CLAUDE.md"), None, 1024, None).await;

    assert_eq!(expansion.files[0].text, "same body");
    assert_eq!(expansion.files.len(), 1);
}

#[tokio::test]
async fn byte_budget_stops_the_expansion() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("big.md"), "0123456789");
    write(&dir.path().join("CLAUDE.md"), "@big.md\n");
    let budget = fs::read(dir.path().join("CLAUDE.md")).unwrap().len();

    let expansion = expand_imports(&dir.path().join("CLAUDE.md"), None, budget, None).await;

    assert!(expansion.files.is_empty());
    assert_eq!(expansion.warnings.len(), 1);
    assert!(expansion.warnings[0].contains("project_doc_max_bytes"));
}

#[tokio::test]
async fn home_prefix_resolves_against_the_given_home() {
    let home = tempfile::tempdir().expect("temp dir");
    let dir = tempfile::tempdir().expect("temp dir");
    write(&home.path().join("shared.md"), "from home\n");
    write(&dir.path().join("CLAUDE.md"), "@~/shared.md\n");

    let expansion = expand_imports(&dir.path().join("CLAUDE.md"), Some(home.path()), 1024, None).await;

    assert_eq!(expansion.files[0].text, "from home");
}

#[tokio::test]
async fn a_cycle_is_included_once() {
    let dir = tempfile::tempdir().expect("temp dir");
    write(&dir.path().join("a.md"), "alpha\n@b.md\n");
    write(&dir.path().join("b.md"), "beta\n@a.md\n");

    let expansion = expand_imports(&dir.path().join("a.md"), None, 1024, None).await;

    assert_eq!(expansion.files.len(), 1);
    assert_eq!(expansion.files[0].text, "alpha\nbeta");
}

/// A file outside the project root must not be read, however the import
/// names it: `~/`, an absolute path, a `..` climb, or a symlink that resolves
/// outside the root.
#[tokio::test]
async fn imports_outside_the_allowed_roots_are_refused() {
    let project = tempfile::tempdir().expect("temp dir");
    let outside = tempfile::tempdir().expect("temp dir");
    let home = tempfile::tempdir().expect("temp dir");
    write(&outside.path().join("secret.md"), "secret\n");
    write(&home.path().join("secret.md"), "secret\n");
    symlink(
        outside.path().join("secret.md"),
        project.path().join("escape.md"),
    )
    .expect("symlink");
    write(
        &project.path().join("CLAUDE.md"),
        "@~/secret.md\n@/etc/hostname\n@../secret.md\n@escape.md\n",
    );
    let roots = vec![project.path().to_path_buf()];

    let expansion = expand_imports(
        &project.path().join("CLAUDE.md"),
        Some(home.path()),
        1024,
        Some(&roots),
    )
    .await;

    let text = expansion
        .files
        .iter()
        .map(|file| file.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    // A refused import stays as the line that named it, so the text still
    // contains the references; only the project file itself may be read.
    assert_eq!(
        expansion.bytes_read,
        fs::read(project.path().join("CLAUDE.md")).unwrap().len(),
        "an import outside the root was read: {text:?}"
    );
    assert!(
        expansion.warnings.len() >= 3,
        "each refused import must be reported, got {:?}",
        expansion.warnings
    );
    assert!(
        expansion.warnings.iter().all(|warning| warning.contains("outside the allowed project roots")),
        "unexpected warning: {:?}",
        expansion.warnings
    );
}

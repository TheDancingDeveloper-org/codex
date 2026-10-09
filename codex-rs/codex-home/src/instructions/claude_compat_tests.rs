use std::fs;

use codex_extension_api::UserInstructionsProvider;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;

use super::ClaudeCompatUserInstructionsProvider;

fn provider(user_home: &std::path::Path, max_bytes: usize) -> ClaudeCompatUserInstructionsProvider {
    let codex_home = tempfile::tempdir().expect("codex home");
    // Leak: the provider only stores the path, and the dir lives until process exit.
    let codex_home = Box::leak(Box::new(codex_home));
    ClaudeCompatUserInstructionsProvider::new(
        AbsolutePathBuf::try_from(codex_home.path().to_path_buf()).expect("absolute"),
        Some(user_home.to_path_buf()),
        max_bytes,
    )
}

#[tokio::test]
async fn claude_md_then_rules_in_sorted_order() {
    let home = tempfile::tempdir().expect("home");
    let claude = home.path().join(".claude");
    fs::create_dir_all(claude.join("rules")).expect("rules dir");
    fs::write(claude.join("CLAUDE.md"), "global claude\n").expect("claude md");
    fs::write(claude.join("rules").join("b.md"), "rule b\n").expect("rule b");
    fs::write(claude.join("rules").join("a.md"), "rule a\n").expect("rule a");
    fs::write(claude.join("rules").join("notes.txt"), "not a rule").expect("txt");

    let loaded = provider(home.path(), 32 * 1024)
        .load_user_instructions()
        .await;

    let text = loaded.instructions.expect("instructions").text;
    let claude_at = text.find("global claude").expect("claude text");
    let a_at = text.find("rule a").expect("rule a");
    let b_at = text.find("rule b").expect("rule b");
    assert!(claude_at < a_at && a_at < b_at, "order was {text:?}");
    assert!(!text.contains("not a rule"));
    assert!(loaded.warnings.is_empty());
}

#[tokio::test]
async fn rules_import_is_expanded_and_deduplicated() {
    let home = tempfile::tempdir().expect("home");
    let claude = home.path().join(".claude");
    fs::create_dir_all(claude.join("rules")).expect("rules dir");
    fs::write(claude.join("shared.md"), "shared once\n").expect("shared");
    fs::write(claude.join("CLAUDE.md"), "@shared.md\n").expect("claude md");
    fs::write(claude.join("rules").join("r.md"), "@../shared.md\n").expect("rule");

    let loaded = provider(home.path(), 32 * 1024)
        .load_user_instructions()
        .await;

    let text = loaded.instructions.expect("instructions").text;
    assert_eq!(text.matches("shared once").count(), 1, "{text:?}");
}

#[tokio::test]
async fn the_budget_covers_claude_files_and_rules_together() {
    let home = tempfile::tempdir().expect("home");
    let claude = home.path().join(".claude");
    fs::create_dir_all(claude.join("rules")).expect("rules dir");
    let claude_md = "0123456789abcdef";
    fs::write(claude.join("CLAUDE.md"), claude_md).expect("claude md");
    fs::write(claude.join("rules").join("r.md"), "rule body").expect("rule");

    let loaded = provider(home.path(), claude_md.len())
        .load_user_instructions()
        .await;

    let text = loaded.instructions.expect("instructions").text;
    assert!(text.contains(claude_md));
    assert!(
        !text.contains("rule body"),
        "rule must be past the budget: {text:?}"
    );
    assert_eq!(loaded.warnings.len(), 1);
}

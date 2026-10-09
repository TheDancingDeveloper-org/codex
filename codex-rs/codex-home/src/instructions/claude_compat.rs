//! Global user instructions with Claude Code compatibility.
//!
//! Fork-owned (WI-1131). Wraps [`CodexHomeUserInstructionsProvider`] and, when
//! enabled, appends `~/.claude/CLAUDE.md` and then `~/.claude/rules/*.md` in
//! sorted order. Both go through the shared `@import` expander, so a line that
//! is only `@path` is replaced by that file, to depth 5, deduplicated by path,
//! symlink and content, and counted against `project_doc_max_bytes`.

use std::path::PathBuf;
use std::sync::Arc;

use codex_extension_api::Instructions;
use codex_extension_api::LoadInstructionsFuture;
use codex_extension_api::LoadedUserInstructions;
use codex_extension_api::UserInstructionsProvider;
use codex_utils_absolute_path::AbsolutePathBuf;

use super::CodexHomeUserInstructionsProvider;
use super::imports::expand_imports_all;

const CLAUDE_DIR: &str = ".claude";
const CLAUDE_MD: &str = "CLAUDE.md";
const RULES_DIR: &str = "rules";

/// Loads the Codex home instructions plus the user's Claude Code files.
#[derive(Clone, Debug)]
pub struct ClaudeCompatUserInstructionsProvider {
    inner: CodexHomeUserInstructionsProvider,
    /// The user's home directory, where `~/.claude` lives.
    user_home: Option<PathBuf>,
    /// Bytes the appended Claude files may consume in total.
    max_bytes: usize,
}

impl ClaudeCompatUserInstructionsProvider {
    /// Creates a provider. `user_home` is the directory holding `.claude`; pass
    /// `None` when it cannot be determined and only the Codex home is loaded.
    pub fn new(
        codex_home: AbsolutePathBuf,
        user_home: Option<PathBuf>,
        max_bytes: usize,
    ) -> Self {
        Self {
            inner: CodexHomeUserInstructionsProvider::new(codex_home),
            user_home,
            max_bytes,
        }
    }

    async fn load(&self) -> LoadedUserInstructions {
        let mut loaded = self.inner.load_user_instructions().await;
        let Some(user_home) = &self.user_home else {
            return loaded;
        };
        let claude_dir = user_home.join(CLAUDE_DIR);
        let mut paths = vec![claude_dir.join(CLAUDE_MD)];
        if let Some(rules) = rule_files(&claude_dir.join(RULES_DIR)).await {
            paths.extend(rules);
        }
        let expanded = expand_imports_all(&paths, Some(user_home), self.max_bytes, None).await;
        loaded.warnings.extend(expanded.warnings);
        if expanded.files.is_empty() {
            return loaded;
        }
        let claude_text = expanded
            .files
            .iter()
            .map(|file| format!("Contents of {}:\n\n{}", file.path.display(), file.text))
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = match loaded.instructions.take() {
            Some(existing) => format!("{}\n\n{claude_text}", existing.text),
            None => claude_text,
        };
        loaded.instructions = Some(Instructions {
            text,
            // The snapshot now spans several files, so no single source path
            // describes it.
            source: None,
        });
        loaded
    }
}

impl UserInstructionsProvider for ClaudeCompatUserInstructionsProvider {
    fn load_user_instructions(&self) -> LoadInstructionsFuture<'_> {
        Box::pin(async move { self.load().await })
    }
}

/// `*.md` files directly inside `dir`, sorted by name. `None` when the
/// directory does not exist.
async fn rule_files(dir: &std::path::Path) -> Option<Vec<PathBuf>> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(_) => return None,
    };
    let mut files = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
    files.sort();
    Some(files)
}

/// Selects the global user-instructions provider for a runtime.
///
/// `claude_compat` is the fork's config switch: off keeps upstream behaviour
/// exactly, on appends the user's `~/.claude` files.
pub fn user_instructions_provider(
    codex_home: AbsolutePathBuf,
    claude_compat: bool,
    max_bytes: usize,
) -> Arc<dyn UserInstructionsProvider> {
    if claude_compat {
        Arc::new(ClaudeCompatUserInstructionsProvider::new(
            codex_home,
            dirs::home_dir(),
            max_bytes,
        ))
    } else {
        Arc::new(CodexHomeUserInstructionsProvider::new(codex_home))
    }
}

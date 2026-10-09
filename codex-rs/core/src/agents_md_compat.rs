//! Fork (WI-1132): the `project_doc_mode = "all"` project-doc loader.
//!
//! Where the upstream discovery keeps the first matching filename in each
//! directory, this loads every matching filename and then `.claude/rules/*.md`
//! in sorted order, and expands line-only `@path` imports. Every file is read
//! through the executor filesystem and, after canonicalization, must fall
//! inside the project root, so a hostile repo cannot pull in `~/.codex`,
//! `/etc`, a `..` climb or a symlink that escapes the project.

use std::collections::HashSet;
use std::hash::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::io;

use codex_file_system::ExecutorFileSystem;
use codex_file_system::FileSystemSandboxContext;
use codex_file_system::GetMetadataOptions;
use codex_file_system::ReadFileOptions;
use codex_utils_path_uri::PathUri;

/// A file the `all` mode wants loaded, in the order it should appear.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CompatDoc {
    pub path: PathUri,
    pub text: String,
}

const RULES_DIR: &str = ".claude/rules";
const MAX_IMPORT_DEPTH: usize = 5;

/// Discovers the docs for one directory.
///
/// `names` is the candidate filename list. Every name that names a file is
/// kept, in the order given, and `.claude/rules/*.md` follows in sorted order.
/// `root` is the project root the caller already computed; a doc or an import
/// that canonicalizes outside it is skipped.
pub(crate) async fn discover_compat_docs(
    fs: &dyn ExecutorFileSystem,
    directory: &PathUri,
    names: &[&str],
    root: &PathUri,
    sandbox: Option<&FileSystemSandboxContext>,
) -> io::Result<Vec<PathUri>> {
    let mut found = Vec::new();
    for name in names {
        let candidate = directory
            .join(name)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
        if is_file(fs, &candidate, sandbox).await? {
            found.push(candidate);
        }
    }

    let rules_dir = directory
        .join(RULES_DIR)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    if let Some(mut rules) = rule_files(fs, &rules_dir, root, sandbox).await? {
        rules.sort_by_key(ToString::to_string);
        found.extend(rules);
    }
    Ok(found)
}

/// Reads `paths` and expands their `@path` imports.
///
/// `budget` is the number of bytes still available and counts every file read,
/// including one reached only through an import. Files are deduped by their
/// canonical path and by their body, and expansion stops at depth 5.
pub(crate) async fn expand_project_docs(
    fs: &dyn ExecutorFileSystem,
    paths: &[PathUri],
    root: &PathUri,
    budget: usize,
    sandbox: Option<&FileSystemSandboxContext>,
) -> io::Result<Vec<CompatDoc>> {
    let mut seen_paths = HashSet::new();
    let mut seen_bodies = HashSet::new();
    let mut docs = Vec::new();
    let mut remaining = budget;
    for path in paths {
        read_doc(
            fs,
            path,
            root,
            /*depth*/ 0,
            sandbox,
            &mut seen_paths,
            &mut seen_bodies,
            &mut docs,
            &mut remaining,
        )
        .await?;
    }
    Ok(docs)
}

#[allow(clippy::too_many_arguments)]
async fn read_doc(
    fs: &dyn ExecutorFileSystem,
    path: &PathUri,
    root: &PathUri,
    depth: usize,
    sandbox: Option<&FileSystemSandboxContext>,
    seen_paths: &mut HashSet<String>,
    seen_bodies: &mut HashSet<u64>,
    docs: &mut Vec<CompatDoc>,
    remaining: &mut usize,
) -> io::Result<()> {
    if *remaining == 0 || depth > MAX_IMPORT_DEPTH {
        return Ok(());
    }
    let canonical = match fs.canonicalize(path, sandbox).await {
        Ok(path) => path,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    if !canonical.starts_with(root) {
        tracing::warn!(path = %canonical, "refusing project doc outside the project root");
        return Ok(());
    }
    if !seen_paths.insert(canonical.to_string()) {
        return Ok(());
    }

    let mut data = fs
        .read_file(&canonical, ReadFileOptions::default(), sandbox)
        .await?;
    if data.len() > *remaining {
        data.truncate(*remaining);
    }
    *remaining = remaining.saturating_sub(data.len());
    let text = String::from_utf8_lossy(&data).to_string();

    let mut body = String::new();
    if depth < MAX_IMPORT_DEPTH {
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(reference) = line_only_import(trimmed) {
                let imported = resolve_import(path, reference)?;
                let before = docs.len();
                Box::pin(read_doc(
                    fs,
                    &imported,
                    root,
                    depth + 1,
                    sandbox,
                    seen_paths,
                    seen_bodies,
                    docs,
                    remaining,
                ))
                .await?;
                if docs.len() == before {
                    body.push_str(line);
                    body.push('\n');
                }
            } else {
                body.push_str(line);
                body.push('\n');
            }
        }
    } else {
        body = text;
    }

    let digest = {
        let mut hasher = DefaultHasher::new();
        body.trim().hash(&mut hasher);
        hasher.finish()
    };
    if body.trim().is_empty() || !seen_bodies.insert(digest) {
        return Ok(());
    }
    docs.push(CompatDoc {
        path: canonical,
        text: body.trim().to_string(),
    });
    Ok(())
}

/// A line that is nothing but an `@path` reference, or `None`.
fn line_only_import(line: &str) -> Option<&str> {
    let reference = line.strip_prefix('@')?.trim();
    if reference.is_empty() || reference.contains(char::is_whitespace) {
        None
    } else {
        Some(reference)
    }
}

fn resolve_import(from: &PathUri, reference: &str) -> io::Result<PathUri> {
    if reference.starts_with('~') || reference.starts_with('/') || reference.contains(':') {
        PathUri::from_host_native_path(reference)
    } else {
        let parent = from
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "import has no parent"))?;
        parent
            .join(reference)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
    }
}

async fn is_file(
    fs: &dyn ExecutorFileSystem,
    path: &PathUri,
    sandbox: Option<&FileSystemSandboxContext>,
) -> io::Result<bool> {
    match fs
        .get_metadata(path, GetMetadataOptions::default(), sandbox)
        .await
    {
        Ok(metadata) => Ok(metadata.is_file),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

async fn rule_files(
    fs: &dyn ExecutorFileSystem,
    rules_dir: &PathUri,
    root: &PathUri,
    sandbox: Option<&FileSystemSandboxContext>,
) -> io::Result<Option<Vec<PathUri>>> {
    let canonical = match fs.canonicalize(rules_dir, sandbox).await {
        Ok(path) => path,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    if !canonical.starts_with(root) {
        tracing::warn!(path = %canonical, "refusing .claude/rules outside the project root");
        return Ok(None);
    }
    let entries = match fs.read_directory(&canonical, sandbox).await {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut files = Vec::new();
    for entry in entries {
        if entry.is_directory || !entry.file_name.ends_with(".md") {
            continue;
        }
        if let Ok(path) = canonical.join(&entry.file_name) {
            files.push(path);
        }
    }
    Ok(Some(files))
}

#[cfg(test)]
#[path = "agents_md_compat_tests.rs"]
mod tests;

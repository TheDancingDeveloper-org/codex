//! Shared `@import` expansion for Claude-compat instruction files.
//!
//! Fork-owned (WI-1131). Both the global user-instructions provider and the
//! per-directory project-doc loader (WI-1132) expand imports through here, so
//! the rules live in one place:
//!
//! - only a line that is nothing but `@path` is an import, and only outside a
//!   fenced code block (` ``` ` or `~~~`);
//! - the path is resolved relative to the importing file, `~/` means the home
//!   directory, and an absolute path is taken as written;
//! - expansion stops at depth 5;
//! - a file reached twice (by path, by symlink, or by identical body) is kept
//!   once, which also ends cycles;
//! - every byte read counts against the caller's remaining budget.

use std::collections::HashSet;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use sha2::Digest;
use sha2::Sha256;

/// Deepest import nesting that is still expanded. A file at this depth is
/// included verbatim; its own imports are left as written.
pub const MAX_IMPORT_DEPTH: usize = 5;

/// One file an expansion read, in the order it was first reached.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedFile {
    /// Canonical path of the file, after symlink resolution.
    pub path: PathBuf,
    /// The file's text after its own imports were expanded.
    pub text: String,
}

/// Outcome of expanding one instruction file.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportExpansion {
    /// Files that contributed text, in first-seen order.
    pub files: Vec<ImportedFile>,
    /// Bytes consumed from the budget, including files that were deduplicated.
    pub bytes_read: usize,
    /// Recoverable problems: a file that could not be read, or a file skipped
    /// because the byte budget was exhausted.
    pub warnings: Vec<String>,
}

/// Expands `@path` imports starting at `path`.
///
/// `budget` is the number of bytes still available; the expansion stops reading
/// once it is spent and reports the skipped file as a warning. `home` resolves
/// `~/` references and may be `None` when no home directory is known.
///
/// `allowed_roots` confines every file the expansion reads. After a path is
/// canonicalized (so a symlink or `..` cannot slip past), it must sit inside
/// one of the roots; anything else is refused and reported as a warning.
/// `None` confines nothing, which is what the global user-instructions path
/// uses. An empty slice confines everything.
pub async fn expand_imports(
    path: &Path,
    home: Option<&Path>,
    budget: usize,
    allowed_roots: Option<&[PathBuf]>,
) -> ImportExpansion {
    let mut expansion = ImportExpansion::default();
    expand_with(
        &mut expansion,
        std::slice::from_ref(&path.to_path_buf()),
        home,
        budget,
        allowed_roots,
    )
    .await;
    expansion
}

/// Expands several files as one snapshot.
///
/// Files share the byte budget and the path and body deduplication, so a file
/// reached from two of them is kept once and every byte counts once. `budget`
/// is the total still available across all of `paths`. `allowed_roots` is the
/// same confinement as [`expand_imports`].
pub async fn expand_imports_all(
    paths: &[PathBuf],
    home: Option<&Path>,
    budget: usize,
    allowed_roots: Option<&[PathBuf]>,
) -> ImportExpansion {
    let mut expansion = ImportExpansion::default();
    expand_with(&mut expansion, paths, home, budget, allowed_roots).await;
    expansion
}

async fn expand_with(
    expansion: &mut ImportExpansion,
    paths: &[PathBuf],
    home: Option<&Path>,
    budget: usize,
    allowed_roots: Option<&[PathBuf]>,
) {
    let mut expander = Expander {
        home: home.map(Path::to_path_buf),
        allowed_roots: canonicalize_roots(allowed_roots).await,
        seen_paths: HashSet::new(),
        seen_bodies: HashSet::new(),
        files: Vec::new(),
        bytes_read: 0,
        budget,
        warnings: Vec::new(),
    };
    for path in paths {
        expander.add(path, 0).await;
    }
    expansion.files.append(&mut expander.files);
    expansion.bytes_read += expander.bytes_read;
    expansion.warnings.append(&mut expander.warnings);
}

/// Canonicalizes each root that exists. A root that does not is dropped, which
/// only ever makes the confinement stricter.
async fn canonicalize_roots(roots: Option<&[PathBuf]>) -> Option<Vec<PathBuf>> {
    let roots = roots?;
    let mut canonical = Vec::with_capacity(roots.len());
    for root in roots {
        if let Ok(path) = tokio::fs::canonicalize(root).await {
            canonical.push(path);
        }
    }
    Some(canonical)
}

struct Expander {
    home: Option<PathBuf>,
    /// Canonical roots a file must fall inside, when confinement is on.
    allowed_roots: Option<Vec<PathBuf>>,
    seen_paths: HashSet<PathBuf>,
    seen_bodies: HashSet<[u8; 32]>,
    files: Vec<ImportedFile>,
    bytes_read: usize,
    budget: usize,
    warnings: Vec<String>,
}

impl Expander {
    /// Loads `path` and what it imports. A file already loaded is skipped.
    async fn add(&mut self, path: &Path, depth: usize) {
        let Some((canonical, text)) = self.read(path).await else {
            return;
        };
        if !self.seen_paths.insert(canonical.clone()) {
            return;
        }
        let index = self.files.len();
        self.files.push(ImportedFile {
            path: canonical.clone(),
            text: String::new(),
        });
        let body = self.expand(&text, canonical.parent(), depth).await;
        let body = body.trim().to_string();
        // A file that is one import which could not be read contributed
        // nothing, so it is not kept. A file that did expand is kept even when
        // its own text matches what it imported.
        let unresolved_import = text.trim().lines().count() == 1
            && line_only_import(text.trim()).is_some()
            && body == text.trim();
        let identity = if unresolved_import {
            format!("{depth}\n{}", text.trim())
        } else {
            text.trim().to_string()
        };
        let digest = Sha256::digest(identity.as_bytes());
        let fresh = self.seen_bodies.insert(digest.into());
        if body.is_empty() || (unresolved_import && depth < MAX_IMPORT_DEPTH) || !fresh {
            self.files.remove(index);
            return;
        }
        self.files[index].text = body;
    }

    /// Whether `path` resolves inside an allowed root.
    ///
    /// Walks up to the nearest ancestor that exists and canonicalizes that, so a
    /// reference to a file that has not been created yet is still confined. With
    /// no roots configured nothing is confined.
    async fn within_allowed_roots(&self, path: &Path) -> bool {
        let Some(roots) = &self.allowed_roots else {
            return true;
        };
        let mut ancestor = path;
        let mut rest = PathBuf::new();
        let canonical = loop {
            if ancestor.as_os_str().is_empty() {
                return false;
            }
            match tokio::fs::canonicalize(ancestor).await {
                Ok(canonical) => break canonical,
                Err(_) => match ancestor.parent() {
                    Some(parent) => {
                        if let Some(name) = ancestor.file_name() {
                            rest = Path::new(name).join(&rest);
                        }
                        ancestor = parent;
                    }
                    None => return false,
                },
            }
        };
        let resolved = canonical.join(rest);
        roots.iter().any(|root| resolved.starts_with(root))
    }

    /// Reads a regular file within the remaining budget.
    async fn read(&mut self, path: &Path) -> Option<(PathBuf, String)> {
        let canonical = match tokio::fs::canonicalize(path).await {
            Ok(path) => path,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                // A missing file is normally not an import. Once confinement is
                // on it is also the way past it: a path that cannot be
                // canonicalized cannot be shown to fall inside a root, and
                // returning None would leave it unreported. Only a path that
                // resolves inside a root may be treated as absent.
                if self.allowed_roots.is_some() && !self.within_allowed_roots(path).await {
                    self.warnings.push(format!(
                        "Refused instructions from `{}`: it is outside the allowed project roots",
                        path.display()
                    ));
                }
                return None;
            }
            Err(err) => {
                self.warnings.push(format!(
                    "Failed to read instructions from `{}`: {err}",
                    path.display()
                ));
                return None;
            }
        };
        if let Some(roots) = &self.allowed_roots
            && !roots.iter().any(|root| canonical.starts_with(root))
        {
            self.warnings.push(format!(
                "Refused instructions from `{}`: it is outside the allowed project roots",
                canonical.display()
            ));
            return None;
        }
        let metadata = match tokio::fs::metadata(&canonical).await {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => return None,
            Err(err) => {
                self.warnings.push(format!(
                    "Failed to read instructions from `{}`: {err}",
                    canonical.display()
                ));
                return None;
            }
        };
        let len = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        if self.bytes_read.saturating_add(len) > self.budget {
            self.warnings.push(format!(
                "Skipped instructions from `{}`: the {} byte project_doc_max_bytes budget is exhausted",
                canonical.display(),
                self.budget
            ));
            return None;
        }
        let data = match tokio::fs::read(&canonical).await {
            Ok(data) => data,
            Err(err) => {
                self.warnings.push(format!(
                    "Failed to read instructions from `{}`: {err}",
                    canonical.display()
                ));
                return None;
            }
        };
        self.bytes_read += data.len();
        Some((canonical, String::from_utf8_lossy(&data).into_owned()))
    }

    /// Strips HTML comments and resolves line-only imports in `text`.
    async fn expand(&mut self, text: &str, dir: Option<&Path>, depth: usize) -> String {
        let text = strip_html_comments(text);
        let mut out = Vec::new();
        let mut in_fence = false;
        for line in text.split('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                out.push(line.to_string());
                continue;
            }
            if in_fence || depth >= MAX_IMPORT_DEPTH {
                out.push(line.to_string());
                continue;
            }
            if let Some(reference) = line_only_import(trimmed)
                && let Some(imported) = Box::pin(self.import_file(reference, dir, depth)).await
            {
                if !imported.is_empty() {
                    out.push(imported);
                }
                continue;
            }
            out.push(line.to_string());
        }
        out.join("\n")
    }

    /// Resolves one `@path` reference. `None` means it does not name a readable
    /// file, so the caller leaves the line as written.
    async fn import_file(
        &mut self,
        reference: &str,
        dir: Option<&Path>,
        depth: usize,
    ) -> Option<String> {
        let path = self.resolve(reference, dir)?;
        let (canonical, text) = self.read(&path).await?;
        if !self.seen_paths.insert(canonical.clone()) {
            return Some(String::new());
        }
        let body = Box::pin(self.expand(&text, canonical.parent(), depth + 1))
            .await
            .trim()
            .to_string();
        if body.is_empty() {
            return Some(String::new());
        }
        // A file that is only an import has no body of its own, so its identity
        // is the depth it was reached at plus what it resolved to. Two links in
        // a chain otherwise hash to the same text and the whole chain collapses.
        let pure_import =
            text.trim().lines().count() == 1 && line_only_import(text.trim()).is_some();
        let identity = if pure_import {
            format!("{}\n{body}", depth + 1)
        } else {
            text.trim().to_string()
        };
        let digest = Sha256::digest(identity.as_bytes());
        if !self.seen_bodies.insert(digest.into()) {
            return Some(String::new());
        }
        Some(body)
    }

    fn resolve(&self, reference: &str, dir: Option<&Path>) -> Option<PathBuf> {
        if let Some(rest) = reference.strip_prefix("~/") {
            return Some(self.home.as_ref()?.join(rest));
        }
        let path = Path::new(reference);
        if path.is_absolute() {
            return Some(path.to_path_buf());
        }
        Some(dir?.join(path))
    }
}

/// A line that is exactly `@path`, with the path characters Klaudia accepts.
fn line_only_import(trimmed: &str) -> Option<&str> {
    let reference = trimmed.strip_prefix('@')?;
    if reference.is_empty()
        || !reference
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '/' | '-' | '~'))
    {
        return None;
    }
    Some(reference)
}

fn strip_html_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

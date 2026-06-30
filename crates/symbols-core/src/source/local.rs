//! Local symbol sources: a directory tree or a recursive glob of debug files,
//! indexed by debug-id on first use. Zero egress.
//!
//! Indexing reads only each candidate's header (memory-mapped) to derive its
//! identity, so building the index over a directory is cheap relative to the
//! full resolve parse that happens lazily on the first matching frame.

use std::path::{Path, PathBuf};

use crate::id::Identity;
use crate::module::probe_identity;

/// One indexed debug file: every id alias it can be looked up by, plus its path.
pub struct Indexed {
    /// Canonical id alias tokens this file answers to.
    pub aliases: Vec<String>,
    /// The concrete file path.
    pub path: PathBuf,
}

/// Memory-map a file and derive its identity aliases, or `None` if it is not a
/// debug file we recognize. Never panics on malformed input.
fn aliases_of(path: &Path) -> Option<Indexed> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: the mapping is read-only and dropped before the function returns;
    // we never retain a borrow of the mapped bytes.
    let mmap = unsafe { memmap2::Mmap::map(&file).ok()? };
    let identity: Identity = probe_identity(&mmap).ok()?;
    Some(Indexed {
        aliases: identity.aliases(),
        path: path.to_path_buf(),
    })
}

/// Recursively index a directory of symbol files. Symlinks are not followed and
/// unreadable entries are skipped. Bounded by `max_files` so a hostile directory
/// cannot make indexing run unbounded.
pub fn index_dir(root: &str, max_files: usize) -> Vec<Indexed> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(root)];
    let mut visited = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            // `root` itself may be a file (a single debug file as a "dir").
            if dir.is_file() {
                if let Some(ix) = aliases_of(&dir) {
                    out.push(ix);
                }
            }
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                visited += 1;
                if visited > max_files {
                    return out;
                }
                if let Some(ix) = aliases_of(&path) {
                    out.push(ix);
                }
            }
        }
    }
    out
}

/// Expand a glob (with `{a,b}` brace alternation) and index every match.
pub fn index_glob(pattern: &str, max_files: usize) -> Vec<Indexed> {
    let mut out = Vec::new();
    let mut visited = 0usize;
    for expanded in expand_braces(pattern) {
        let Ok(paths) = glob::glob(&expanded) else {
            continue;
        };
        for entry in paths.flatten() {
            if entry.is_dir() {
                continue;
            }
            visited += 1;
            if visited > max_files {
                return out;
            }
            if let Some(ix) = aliases_of(&entry) {
                out.push(ix);
            }
        }
    }
    out
}

/// Expand `{a,b,c}` brace alternations in a glob pattern into concrete patterns
/// (`glob` does not support braces). A single level of (possibly multiple)
/// non-nested groups is handled, which covers the `*.{debug,pdb,dSYM}` idiom.
pub fn expand_braces(pattern: &str) -> Vec<String> {
    let mut results = vec![String::new()];
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            let mut group = String::new();
            let mut depth = 1;
            for gc in chars.by_ref() {
                if gc == '{' {
                    depth += 1;
                } else if gc == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                group.push(gc);
            }
            let alts: Vec<&str> = group.split(',').collect();
            let mut next = Vec::with_capacity(results.len() * alts.len());
            for prefix in &results {
                for alt in &alts {
                    next.push(format!("{prefix}{alt}"));
                }
            }
            results = next;
        } else {
            for r in &mut results {
                r.push(c);
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brace_expansion() {
        let mut got = expand_braces("/x/*.{debug,pdb,dSYM}");
        got.sort();
        assert_eq!(
            got,
            vec![
                "/x/*.dSYM".to_string(),
                "/x/*.debug".to_string(),
                "/x/*.pdb".to_string(),
            ]
        );
    }

    #[test]
    fn no_braces_passthrough() {
        assert_eq!(expand_braces("/a/b/*.so"), vec!["/a/b/*.so".to_string()]);
    }
}

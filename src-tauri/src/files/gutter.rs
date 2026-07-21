use std::path::Path;

use serde::Serialize;

use super::Context;
use crate::worktree::diff::{FileHunks, LineKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GutterKind {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GutterMarker {
    pub line: u32,
    pub kind: GutterKind,
}

/// Deriva marcadores de gutter (linha do arquivo em disco → add/mod/del) a
/// partir dos hunks unificados. Regra padrão de SCM: num bloco de mudança, se há
/// adições e remoções, as linhas novas são "modificadas"; só adições → added; só
/// remoções → um marcador de deleted na linha que passou a ocupar a posição.
pub fn markers_from_hunks(hunks: &FileHunks) -> Vec<GutterMarker> {
    if hunks.binary {
        return Vec::new();
    }
    let mut out = Vec::new();
    for hunk in &hunks.hunks {
        let mut new_line = hunk.new_start;
        let mut i = 0;
        while i < hunk.lines.len() {
            match hunk.lines[i].kind {
                LineKind::Context => {
                    new_line += 1;
                    i += 1;
                }
                LineKind::Add | LineKind::Del => {
                    let mut added: Vec<u32> = Vec::new();
                    let mut deleted = 0u32;
                    while i < hunk.lines.len() {
                        match hunk.lines[i].kind {
                            LineKind::Add => {
                                added.push(new_line);
                                new_line += 1;
                            }
                            LineKind::Del => deleted += 1,
                            LineKind::Context => break,
                        }
                        i += 1;
                    }
                    if !added.is_empty() {
                        let kind = if deleted > 0 {
                            GutterKind::Modified
                        } else {
                            GutterKind::Added
                        };
                        for line in added {
                            out.push(GutterMarker { line, kind });
                        }
                    } else if deleted > 0 {
                        let line = new_line.max(1);
                        out.push(GutterMarker {
                            line,
                            kind: GutterKind::Deleted,
                        });
                    }
                }
            }
        }
    }
    out
}

pub fn compute(root: &Path, context: &Context, rel: &str) -> Vec<GutterMarker> {
    let hunks = match context {
        Context::AgentWorktree { base_ref } => {
            crate::worktree::diff::range_file_hunks(root, base_ref, rel, None)
        }
        Context::Repo => crate::worktree::diff::range_file_hunks(root, "HEAD", rel, None),
        Context::OutsideRepo => return Vec::new(),
    };
    match hunks {
        Ok(hunks) => markers_from_hunks(&hunks),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::diff::parse_unified_hunks;

    #[test]
    fn pure_additions_are_marked_added() {
        let diff = "@@ -1,2 +1,4 @@\n line a\n+new one\n+new two\n line b\n";
        let markers = markers_from_hunks(&parse_unified_hunks(diff));
        assert_eq!(
            markers,
            vec![
                GutterMarker {
                    line: 2,
                    kind: GutterKind::Added
                },
                GutterMarker {
                    line: 3,
                    kind: GutterKind::Added
                },
            ]
        );
    }

    #[test]
    fn replaced_lines_are_marked_modified() {
        let diff = "@@ -1,3 +1,3 @@\n keep\n-old\n+new\n tail\n";
        let markers = markers_from_hunks(&parse_unified_hunks(diff));
        assert_eq!(
            markers,
            vec![GutterMarker {
                line: 2,
                kind: GutterKind::Modified
            }]
        );
    }

    #[test]
    fn pure_deletion_marks_deleted_on_following_line() {
        let diff = "@@ -1,3 +1,2 @@\n keep\n-gone\n tail\n";
        let markers = markers_from_hunks(&parse_unified_hunks(diff));
        assert_eq!(
            markers,
            vec![GutterMarker {
                line: 2,
                kind: GutterKind::Deleted
            }]
        );
    }

    #[test]
    fn binary_diff_yields_no_markers() {
        let hunks = FileHunks {
            binary: true,
            hunks: Vec::new(),
        };
        assert!(markers_from_hunks(&hunks).is_empty());
    }
}

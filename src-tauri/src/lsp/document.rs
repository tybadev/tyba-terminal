use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ContentChange {
    #[serde(default)]
    pub range: Option<Range>,
    pub text: String,
}

pub fn byte_offset(text: &str, pos: Position) -> usize {
    let mut offset = 0usize;
    let mut line = 0u32;
    let bytes = text.as_bytes();
    while line < pos.line {
        match memchr(b'\n', &bytes[offset..]) {
            Some(rel) => offset += rel + 1,
            None => return text.len(),
        }
        line += 1;
    }
    let line_end = match memchr(b'\n', &bytes[offset..]) {
        Some(rel) => offset + rel,
        None => text.len(),
    };
    let mut units = 0u32;
    let mut cursor = offset;
    for ch in text[offset..line_end].chars() {
        if units >= pos.character {
            break;
        }
        units += ch.len_utf16() as u32;
        cursor += ch.len_utf8();
    }
    cursor
}

fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    haystack.iter().position(|b| *b == needle)
}

pub fn apply_changes(text: &str, changes: &[ContentChange]) -> String {
    let mut doc = text.to_string();
    for change in changes {
        match change.range {
            None => doc = change.text.clone(),
            Some(range) => {
                let start = byte_offset(&doc, range.start);
                let end = byte_offset(&doc, range.end).max(start);
                doc.replace_range(start..end, &change.text);
            }
        }
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn change(range: Option<Range>, text: &str) -> ContentChange {
        ContentChange {
            range,
            text: text.to_string(),
        }
    }

    #[test]
    fn insert_at_start_of_a_line() {
        let out = apply_changes(
            "fn main() {}\n",
            &[change(
                Some(Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                }),
                "pub ",
            )],
        );
        assert_eq!(out, "pub fn main() {}\n");
    }

    #[test]
    fn replace_across_multiple_lines() {
        let doc = "line one\nline two\nline three\n";
        let out = apply_changes(
            doc,
            &[change(
                Some(Range {
                    start: pos(0, 5),
                    end: pos(2, 4),
                }),
                "X",
            )],
        );
        assert_eq!(out, "line X three\n");
    }

    #[test]
    fn delete_a_range() {
        let out = apply_changes(
            "hello world\n",
            &[change(
                Some(Range {
                    start: pos(0, 5),
                    end: pos(0, 11),
                }),
                "",
            )],
        );
        assert_eq!(out, "hello\n");
    }

    #[test]
    fn sequential_edits_reproduce_the_document() {
        let mut doc = String::new();
        let steps = [
            (
                Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                "l",
            ),
            (
                Range {
                    start: pos(0, 1),
                    end: pos(0, 1),
                },
                "e",
            ),
            (
                Range {
                    start: pos(0, 2),
                    end: pos(0, 2),
                },
                "t",
            ),
            (
                Range {
                    start: pos(0, 3),
                    end: pos(0, 3),
                },
                " x",
            ),
            (
                Range {
                    start: pos(0, 0),
                    end: pos(0, 0),
                },
                "\n",
            ),
            (
                Range {
                    start: pos(0, 0),
                    end: pos(1, 0),
                },
                "",
            ),
        ];
        for (range, text) in steps {
            doc = apply_changes(&doc, &[change(Some(range), text)]);
        }
        assert_eq!(doc, "let x");
    }

    #[test]
    fn full_replace_when_range_is_absent() {
        let out = apply_changes("old contents", &[change(None, "brand new")]);
        assert_eq!(out, "brand new");
    }

    #[test]
    fn utf16_columns_land_after_astral_characters_not_mid_rune() {
        let out = apply_changes(
            "😀ab",
            &[change(
                Some(Range {
                    start: pos(0, 2),
                    end: pos(0, 2),
                }),
                "X",
            )],
        );
        assert_eq!(
            out, "😀Xab",
            "coluna 2 são 2 unidades UTF-16 do emoji: o offset precisa pular os 4 bytes"
        );
    }

    #[test]
    fn multibyte_columns_are_measured_in_code_units_not_bytes() {
        let out = apply_changes(
            "café\n",
            &[change(
                Some(Range {
                    start: pos(0, 4),
                    end: pos(0, 4),
                }),
                "!",
            )],
        );
        assert_eq!(out, "café!\n");
    }

    #[test]
    fn character_past_line_end_clamps() {
        let out = apply_changes(
            "ab\ncd\n",
            &[change(
                Some(Range {
                    start: pos(0, 99),
                    end: pos(0, 99),
                }),
                "Z",
            )],
        );
        assert_eq!(out, "abZ\ncd\n");
    }

    #[test]
    fn line_past_document_end_clamps_to_end() {
        let out = apply_changes(
            "one\n",
            &[change(
                Some(Range {
                    start: pos(9, 0),
                    end: pos(9, 0),
                }),
                "two",
            )],
        );
        assert_eq!(out, "one\ntwo");
    }
}

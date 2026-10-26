//! Problems with Mermaid source, located by line and column.

/// One problem with the Mermaid text: where it is and what is wrong. A diagram with any
/// issue is not drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramIssue {
    /// 1-based.
    pub line: u32,
    /// 1-based, in characters (not bytes).
    pub column: u32,
    /// The whole source line the problem is on, without its line break.
    pub excerpt: String,
    pub message: String,
}

impl std::fmt::Display for DiagramIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

/// Character offsets to lines and columns.
#[derive(Debug)]
pub(crate) struct Lines {
    /// The character offset each line starts at.
    starts: Vec<usize>,
    text: Vec<char>,
}

impl Lines {
    pub fn new(text: &[char]) -> Self {
        let mut starts = vec![0];
        for (i, &c) in text.iter().enumerate() {
            if c == '\n' {
                starts.push(i + 1);
            }
        }
        Self {
            starts,
            text: text.to_vec(),
        }
    }

    /// An issue at character offset `at`.
    pub fn issue(&self, at: usize, message: impl Into<String>) -> DiagramIssue {
        let line = self.starts.partition_point(|&s| s <= at).max(1) - 1;
        let start = self.starts[line];
        let end = self
            .starts
            .get(line + 1)
            .map_or(self.text.len(), |&next| next - 1);
        let excerpt: String = self.text[start..end.max(start)]
            .iter()
            .collect::<String>()
            .trim_end_matches('\r')
            .to_owned();
        DiagramIssue {
            line: line as u32 + 1,
            column: (at.saturating_sub(start)) as u32 + 1,
            excerpt,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_become_lines_and_columns() {
        let text: Vec<char> = "ab\ncdé\n\nx".chars().collect();
        let lines = Lines::new(&text);
        let i = lines.issue(0, "m");
        assert_eq!((i.line, i.column, i.excerpt.as_str()), (1, 1, "ab"));
        let i = lines.issue(5, "m"); // 'é'
        assert_eq!((i.line, i.column, i.excerpt.as_str()), (2, 3, "cdé"));
        let i = lines.issue(7, "m"); // the empty line
        assert_eq!((i.line, i.column, i.excerpt.as_str()), (3, 1, ""));
        let i = lines.issue(9, "m"); // end of text
        assert_eq!((i.line, i.column, i.excerpt.as_str()), (4, 2, "x"));
    }
}

//! The Mermaid `flowchart` subset Facetry reads (`graph` is the same thing).
//!
//! Supported:
//!
//! * the header `flowchart` or `graph`, with a direction `TB`, `TD`, `BT`, `LR` or `RL`
//!   (default `TB`);
//! * statements separated by line breaks or `;`, and `%%` comment lines;
//! * nodes `id`, with a shape and label: `[rect]`, `(rounded)`, `([stadium])`,
//!   `[(cylinder)]`, `((circle))`, `{diamond}`, `{{hexagon}}`, `[/parallelogram/]`,
//!   `[\parallelogram alt\]`, `[/trapezoid\]`, `[\trapezoid alt/]`. A label may be quoted
//!   (`["a [b]"]`), may contain `<br>` line breaks and the entities `#quot;`, `#amp;`, `#lt;`,
//!   `#gt;` and `#<number>;`. A node's last shape and label win;
//! * edges `-->`, `---`, `-.->`, `-.-`, `==>`, `===`, with circle (`--o`) or cross (`--x`)
//!   heads, two-headed forms (`<-->`, `o--o`, `x--x`), extra length (`--->`, `-..->`,
//!   `====>`), and labels (`-->|text|`, `-- text -->`, `-. text .->`, `== text ==>`);
//! * chains `a --> b --> c` and groups `a & b --> c & d`;
//! * icons: `a@{ icon: "bulb" }` (alone or after a shape: `a([Idea])@{ icon: "fa:lightbulb" }`),
//!   one of Facetry's built-in icons or of the user's own ([`parse_with_icons`]);
//! * styling: `classDef name,other fill:#f96,stroke:#333,stroke-width:2px,color:#fff` (and
//!   `classDef default ...` for every node), `class a,b name`, `a:::name`, and
//!   `style a fill:#f96`; later styles win over earlier ones, `style` over classes.
//!
//! Everything else that is valid Mermaid (subgraphs, `linkStyle`, other style properties,
//! click handlers, keys of `@{ }` other than `icon`, front matter and directives, ...) is an
//! error that names the feature, so a diagram is never drawn with parts silently missing. All problems are
//! reported, not only the first: after an error the parser skips to the next statement.

use facetry_core::spec::{EdgeEnd, EdgeStroke, NodeShape};

use crate::issue::{DiagramIssue, Lines};
use crate::style::{self, NodeStyle};

/// Which way ranks run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Direction {
    /// Top to bottom (`TB`, `TD`).
    #[default]
    TopBottom,
    BottomTop,
    LeftRight,
    RightLeft,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    /// Lines separated by `\n`.
    pub label: String,
    pub shape: NodeShape,
    /// What `classDef`, `class`, `:::` and `style` made of it.
    pub style: NodeStyle,
    /// A built-in icon, from `@{ icon: "..." }`.
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// Indices into [`Flowchart::nodes`].
    pub from: usize,
    pub to: usize,
    pub label: Option<String>,
    pub stroke: EdgeStroke,
    pub head: EdgeEnd,
    pub tail: EdgeEnd,
    /// How many ranks the edge spans at least: 1, more for `--->` and friends.
    pub length: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Flowchart {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Styling statements, resolved once the whole text is read (a class may be defined after
/// the nodes that use it).
#[derive(Debug, Default)]
struct Styling {
    classes: Vec<(String, NodeStyle)>,
    /// `(node, class, offset)` in the order written.
    uses: Vec<(String, String, usize)>,
    /// `(node, style, offset)` from `style` statements.
    styles: Vec<(String, NodeStyle, usize)>,
}

/// The longest edge Mermaid's extra dashes can ask for.
const MAX_LENGTH: u32 = 10;

/// Mermaid diagram types other than the three Facetry draws.
const OTHER_TYPES: &[&str] = &[
    "classDiagram",
    "classDiagram-v2",
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "gantt",
    "pie",
    "journey",
    "gitGraph",
    "mindmap",
    "timeline",
    "quadrantChart",
    "requirementDiagram",
    "sankey-beta",
    "xychart-beta",
    "block-beta",
    "packet-beta",
    "kanban",
    "radar-beta",
    "C4Context",
    "C4Container",
    "C4Component",
    "C4Dynamic",
    "C4Deployment",
    "zenuml",
];

/// Statement keywords of flowcharts that are not supported yet, with what to say.
const UNSUPPORTED_STATEMENTS: &[(&str, &str)] = &[
    ("subgraph", "subgraphs are not supported yet"),
    (
        "end",
        "`end` closes a subgraph, and subgraphs are not supported yet",
    ),
    (
        "direction",
        "`direction` (inside subgraphs) is not supported yet",
    ),
    ("linkStyle", "edge styling (`linkStyle`) is not supported"),
    ("click", "click handlers (`click`) are not supported"),
    ("callback", "click handlers (`callback`) are not supported"),
    (
        "accTitle",
        "accessibility titles (`accTitle`) are not supported",
    ),
    (
        "accDescr",
        "accessibility descriptions (`accDescr`) are not supported",
    ),
];

/// What kind of diagram the source is, from its first meaningful word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Flowchart,
    Sequence,
    Architecture,
}

struct Parser<'a> {
    text: &'a [char],
    lines: &'a Lines,
    /// Names of the user's own icons, which `@{ icon }` may use besides the built-in ones.
    icons: &'a [&'a str],
    pos: usize,
    issues: Vec<DiagramIssue>,
    chart: Flowchart,
    styling: Styling,
}

/// A failed statement: the issue is already recorded; skip to the next statement.
struct Skip;

type Step<T> = Result<T, Skip>;

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Parses a whole flowchart, or every problem found in it.
pub fn parse(source: &str) -> Result<Flowchart, Vec<DiagramIssue>> {
    parse_with_icons(source, &[])
}

/// [`parse`], where `@{ icon }` may also name one of `icons` (the user's own icons).
pub fn parse_with_icons(source: &str, icons: &[&str]) -> Result<Flowchart, Vec<DiagramIssue>> {
    let text: Vec<char> = source.chars().collect();
    let lines = Lines::new(&text);
    let mut p = Parser {
        text: &text,
        lines: &lines,
        icons,
        pos: 0,
        issues: Vec::new(),
        chart: Flowchart::default(),
        styling: Styling::default(),
    };
    p.document();
    if p.issues.is_empty() {
        Ok(p.chart)
    } else {
        Err(p.issues)
    }
}

/// The diagram type of `source` and the character offset of its header, or the problem.
pub fn kind(source: &str) -> Result<Kind, DiagramIssue> {
    let text: Vec<char> = source.chars().collect();
    let lines = Lines::new(&text);
    let mut p = Parser {
        text: &text,
        lines: &lines,
        icons: &[],
        pos: 0,
        issues: Vec::new(),
        chart: Flowchart::default(),
        styling: Styling::default(),
    };
    p.skip_preamble();
    if let Some(issue) = std::mem::take(&mut p.issues).into_iter().next() {
        return Err(issue);
    }
    let start = p.pos;
    let word = p.word();
    match word.as_str() {
        "flowchart" | "graph" => Ok(Kind::Flowchart),
        "sequenceDiagram" => Ok(Kind::Sequence),
        "architecture-beta" => Ok(Kind::Architecture),
        "" if start >= text.len() => Err(lines.issue(
            start,
            "empty diagram: expected a diagram type such as `flowchart`",
        )),
        other if OTHER_TYPES.contains(&other) => Err(lines.issue(
            start,
            format!(
                "`{other}` diagrams are not supported; Facetry draws `flowchart`, `sequenceDiagram` and `architecture-beta`"
            ),
        )),
        _ => Err(lines.issue(
            start,
            "expected a diagram type such as `flowchart` or `graph`",
        )),
    }
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.text.get(self.pos).copied()
    }

    fn peek_at(&self, k: usize) -> Option<char> {
        self.text.get(self.pos + k).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(k, c)| self.peek_at(k) == Some(c))
    }

    fn fail<T>(&mut self, at: usize, message: impl Into<String>) -> Step<T> {
        self.issues.push(self.lines.issue(at, message));
        Err(Skip)
    }

    /// Spaces and tabs.
    fn skip_inline_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r')) {
            self.pos += 1;
        }
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' {
                break;
            }
            self.pos += 1;
        }
    }

    /// A run of letters, digits, `_` and `-` (diagram type names have dashes).
    fn word(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|c| is_id_char(c) || c == '-') {
            self.pos += 1;
        }
        self.text[start..self.pos].iter().collect()
    }

    /// Blank lines, `%%` comments, and the front matter and directives Facetry rejects.
    fn skip_preamble(&mut self) {
        loop {
            self.skip_inline_ws();
            match self.peek() {
                Some('\n') => self.pos += 1,
                Some('%') if self.starts_with("%%{") => {
                    let at = self.pos;
                    let _: Step<()> = self.fail(at, "directives (`%%{ ... }%%`) are not supported");
                    self.skip_line();
                }
                Some('%') if self.starts_with("%%") => self.skip_line(),
                Some('-') if self.starts_with("---") => {
                    let at = self.pos;
                    let _: Step<()> = self.fail(at, "front matter (`---`) is not supported");
                    // Skip to the closing `---`.
                    self.skip_line();
                    loop {
                        if self.peek().is_none() {
                            break;
                        }
                        self.pos += 1; // the line break
                        self.skip_inline_ws();
                        let closing = self.starts_with("---");
                        self.skip_line();
                        if closing {
                            break;
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn document(&mut self) {
        self.skip_preamble();
        let start = self.pos;
        let word = self.word();
        if word != "flowchart" && word != "graph" {
            self.issues
                .push(self.lines.issue(start, "expected `flowchart` or `graph`"));
            return;
        }
        self.skip_inline_ws();
        let at = self.pos;
        let dir = self.word();
        self.chart.direction = match dir.as_str() {
            "" => Direction::TopBottom,
            "TB" | "TD" => Direction::TopBottom,
            "BT" => Direction::BottomTop,
            "LR" => Direction::LeftRight,
            "RL" => Direction::RightLeft,
            other => {
                let _: Step<()> = self.fail(
                    at,
                    format!("unknown direction `{other}`: expected `TB`, `TD`, `BT`, `LR` or `RL`"),
                );
                self.skip_line();
                Direction::TopBottom
            }
        };
        self.statements();
        self.resolve_styles();
    }

    /// Applies `classDef default`, then each node's classes, then its `style` statements.
    fn resolve_styles(&mut self) {
        let styling = std::mem::take(&mut self.styling);
        let class = |name: &str| styling.classes.iter().rev().find(|(n, _)| n == name);
        if let Some((_, default)) = class("default") {
            for node in &mut self.chart.nodes {
                node.style.apply(default);
            }
        }
        for (node, name, at) in &styling.uses {
            let Some((_, style)) = class(name) else {
                let _: Step<()> = self.fail(*at, format!("no `classDef` named `{name}`"));
                continue;
            };
            match self.chart.nodes.iter_mut().find(|n| &n.id == node) {
                Some(n) => n.style.apply(style),
                None => {
                    let _: Step<()> = self.fail(*at, format!("no node `{node}` to style"));
                }
            }
        }
        for (node, style, at) in &styling.styles {
            match self.chart.nodes.iter_mut().find(|n| &n.id == node) {
                Some(n) => n.style.apply(style),
                None => {
                    let _: Step<()> = self.fail(*at, format!("no node `{node}` to style"));
                }
            }
        }
    }

    /// A comma-separated list of names (`a,b,c`), each made of id characters.
    fn names(&mut self) -> Step<Vec<(String, usize)>> {
        let mut out = Vec::new();
        loop {
            self.skip_inline_ws();
            let at = self.pos;
            while self.peek().is_some_and(|c| is_id_char(c) || c == '-') {
                self.pos += 1;
            }
            let name: String = self.text[at..self.pos].iter().collect();
            if name.is_empty() {
                return self.fail(at, "expected a name");
            }
            out.push((name, at));
            self.skip_inline_ws();
            if self.peek() == Some(',') {
                self.pos += 1;
            } else {
                return Ok(out);
            }
        }
    }

    /// The rest of the statement as a style property list.
    fn properties(&mut self) -> Step<NodeStyle> {
        self.skip_inline_ws();
        let at = self.pos;
        while !self.at_statement_end() {
            self.pos += 1;
        }
        let text: String = self.text[at..self.pos].iter().collect();
        if text.trim().is_empty() {
            return self.fail(at, "expected style properties such as `fill:#f96`");
        }
        style::parse(text.trim_end()).map_err(|(offset, message)| {
            let offset = text[..offset.min(text.len())].chars().count();
            self.issues.push(self.lines.issue(at + offset, message));
            Skip
        })
    }

    /// `classDef`, `class` and `style`, after their keyword.
    fn styling_statement(&mut self, keyword: &str) -> Step<()> {
        match keyword {
            "classDef" => {
                let names = self.names()?;
                let style = self.properties()?;
                for (name, _) in names {
                    self.styling.classes.push((name, style.clone()));
                }
            }
            "class" => {
                let nodes = self.names()?;
                let classes = self.names()?;
                for (class, at) in classes {
                    for (node, _) in &nodes {
                        self.styling.uses.push((node.clone(), class.clone(), at));
                    }
                }
            }
            _ => {
                let nodes = self.names()?;
                let style = self.properties()?;
                for (node, at) in nodes {
                    self.styling.styles.push((node, style.clone(), at));
                }
            }
        }
        if !self.at_statement_end() {
            let at = self.pos;
            return self.fail(at, "unexpected text after the statement");
        }
        Ok(())
    }

    fn statements(&mut self) {
        loop {
            self.skip_inline_ws();
            match self.peek() {
                None => return,
                Some('\n' | ';') => {
                    self.pos += 1;
                    continue;
                }
                Some('%') if self.starts_with("%%") => {
                    self.skip_line();
                    continue;
                }
                _ => {}
            }
            if self.statement().is_err() {
                self.skip_statement();
            }
        }
    }

    /// After an error: on to the next `;` or line.
    fn skip_statement(&mut self) {
        while let Some(c) = self.peek() {
            if c == '\n' || c == ';' {
                break;
            }
            self.pos += 1;
        }
    }

    fn at_statement_end(&self) -> bool {
        matches!(self.peek(), None | Some('\n' | ';')) || self.starts_with("%%")
    }

    fn statement(&mut self) -> Step<()> {
        // Keywords this subset does not support.
        let start = self.pos;
        let mut end = start;
        while self.text.get(end).copied().is_some_and(is_id_char) {
            end += 1;
        }
        let word: String = self.text[start..end].iter().collect();
        let next = self.text.get(end).copied();
        // A styling keyword followed by a name; `class --> b` is a node called `class`.
        let mut after = end;
        while matches!(self.text.get(after), Some(' ' | '\t')) {
            after += 1;
        }
        let names_follow = self.text.get(after).is_some_and(|&c| is_id_char(c));
        if matches!(word.as_str(), "classDef" | "class" | "style")
            && matches!(next, Some(' ' | '\t'))
            && names_follow
        {
            self.pos = end;
            return self.styling_statement(&word);
        }
        if let Some((_, message)) = UNSUPPORTED_STATEMENTS.iter().find(|(k, _)| *k == word) {
            // `end` and `class` are also fine node ids in front of a shape or link... except
            // that Mermaid itself rejects `end`, and `class` followed by a space is a statement.
            if !matches!(next, Some('[' | '(' | '{' | '>' | '-' | '=' | '&')) {
                return self.fail(start, *message);
            }
        }

        let mut group = self.node_group()?;
        loop {
            self.skip_inline_ws();
            if self.at_statement_end() {
                return Ok(());
            }
            let link = self.link()?;
            self.skip_inline_ws();
            let next = self.node_group()?;
            for &a in &group {
                for &b in &next {
                    let (from, to) = (a, b);
                    self.chart.edges.push(Edge {
                        from,
                        to,
                        label: link.label.clone(),
                        stroke: link.stroke,
                        head: link.head,
                        tail: link.tail,
                        length: link.length,
                    });
                }
            }
            group = next;
        }
    }

    /// `a`, or `a & b & c`.
    fn node_group(&mut self) -> Step<Vec<usize>> {
        let mut out = vec![self.node()?];
        loop {
            let save = self.pos;
            self.skip_inline_ws();
            if self.peek() == Some('&') {
                self.pos += 1;
                self.skip_inline_ws();
                out.push(self.node()?);
            } else {
                self.pos = save;
                return Ok(out);
            }
        }
    }

    fn node(&mut self) -> Step<usize> {
        let start = self.pos;
        while self.peek().is_some_and(is_id_char) {
            self.pos += 1;
        }
        let id: String = self.text[start..self.pos].iter().collect();
        if id.is_empty() {
            return match self.peek() {
                None | Some('\n' | ';') => self.fail(start, "expected a node after the link"),
                Some(c) => self.fail(start, format!("expected a node id, found `{c}`")),
            };
        }
        if id == "end" {
            return self.fail(start, "`end` cannot be a node id (Mermaid reserves it)");
        }
        let shape = self.shape()?;
        let icon = if self.starts_with("@{") {
            self.metadata()?
        } else {
            None
        };
        let mut classes = Vec::new();
        while self.starts_with(":::") {
            self.pos += 3;
            let at = self.pos;
            while self.peek().is_some_and(|c| is_id_char(c) || c == '-') {
                self.pos += 1;
            }
            let class: String = self.text[at..self.pos].iter().collect();
            if class.is_empty() {
                return self.fail(at, "expected a class name after `:::`");
            }
            classes.push((class, at));
        }
        for (class, at) in classes {
            self.styling.uses.push((id.clone(), class, at));
        }
        let index = match self.chart.nodes.iter().position(|n| n.id == id) {
            Some(i) => i,
            None => {
                self.chart.nodes.push(Node {
                    label: id.clone(),
                    id: id.clone(),
                    shape: NodeShape::Rect,
                    style: NodeStyle::default(),
                    icon: None,
                });
                self.chart.nodes.len() - 1
            }
        };
        if icon.is_some() {
            self.chart.nodes[index].icon = icon;
        }
        if let Some((shape, label)) = shape {
            let node = &mut self.chart.nodes[index];
            node.shape = shape;
            node.label = label;
        }
        Ok(index)
    }

    /// Node metadata, `@{ icon: "bulb" }`: of Mermaid's keys only `icon` is read, naming one of
    /// the built-in icons (`fa:`-style prefixes are dropped). Returns the icon's name as
    /// written.
    fn metadata(&mut self) -> Step<Option<String>> {
        let open = self.pos;
        self.pos += 2;
        let mut icon = None;
        loop {
            self.skip_inline_ws();
            match self.peek() {
                Some('}') => {
                    self.pos += 1;
                    return Ok(icon);
                }
                Some(',') => {
                    self.pos += 1;
                    continue;
                }
                None | Some('\n') => return self.fail(open, "unclosed `@{`: expected `}`"),
                _ => {}
            }
            let key_at = self.pos;
            while self.peek().is_some_and(is_id_char) {
                self.pos += 1;
            }
            let key: String = self.text[key_at..self.pos].iter().collect();
            self.skip_inline_ws();
            if self.peek() != Some(':') {
                return self.fail(self.pos, "expected `key: value` in `@{ }`");
            }
            self.pos += 1;
            self.skip_inline_ws();
            let value_at = self.pos;
            let value: String = if self.peek() == Some('"') {
                self.pos += 1;
                let start = self.pos;
                while self.peek().is_some_and(|c| c != '"' && c != '\n') {
                    self.pos += 1;
                }
                if self.peek() != Some('"') {
                    return self.fail(value_at, "unclosed `\"`");
                }
                let v = self.text[start..self.pos].iter().collect();
                self.pos += 1;
                v
            } else {
                let start = self.pos;
                while self
                    .peek()
                    .is_some_and(|c| !matches!(c, ',' | '}' | '\n') && !c.is_whitespace())
                {
                    self.pos += 1;
                }
                self.text[start..self.pos].iter().collect()
            };
            match key.as_str() {
                "icon" => {
                    if !self.icons.contains(&value.as_str())
                        && facetry_core::icons::resolve(&value).is_none()
                    {
                        let names: Vec<&str> = facetry_core::icons::names().collect();
                        let given = if self.icons.is_empty() {
                            String::new()
                        } else {
                            format!(" (nor among the icons given: {})", self.icons.join(", "))
                        };
                        return self.fail(
                            value_at,
                            format!(
                                "no built-in icon `{value}`{given}; the built-in icons are {}",
                                names.join(", ")
                            ),
                        );
                    }
                    icon = Some(value);
                }
                _ => {
                    return self.fail(
                        key_at,
                        format!("`{key}` is not supported in `@{{ }}` (only `icon`)"),
                    );
                }
            }
        }
    }

    /// A shape and its label right after a node id, if there is one.
    fn shape(&mut self) -> Step<Option<(NodeShape, String)>> {
        let at = self.pos;
        let unsupported = |name: &str| format!("{name} nodes are not supported");
        // Longest openers first.
        let (open_len, shape, closers): (usize, NodeShape, &[(&str, NodeShape)]) =
            if self.starts_with("(((") {
                return self.fail(at, unsupported("double circle (`(((...)))`)"));
            } else if self.starts_with("((") {
                (2, NodeShape::Circle, &[("))", NodeShape::Circle)])
            } else if self.starts_with("([") {
                (2, NodeShape::Stadium, &[("])", NodeShape::Stadium)])
            } else if self.starts_with("(") {
                (1, NodeShape::Rounded, &[(")", NodeShape::Rounded)])
            } else if self.starts_with("[[") {
                return self.fail(at, unsupported("subroutine (`[[...]]`)"));
            } else if self.starts_with("[(") {
                (2, NodeShape::Cylinder, &[(")]", NodeShape::Cylinder)])
            } else if self.starts_with("[/") {
                (
                    2,
                    NodeShape::Parallelogram,
                    &[
                        ("/]", NodeShape::Parallelogram),
                        ("\\]", NodeShape::Trapezoid),
                    ],
                )
            } else if self.starts_with("[\\") {
                (
                    2,
                    NodeShape::ParallelogramAlt,
                    &[
                        ("\\]", NodeShape::ParallelogramAlt),
                        ("/]", NodeShape::TrapezoidAlt),
                    ],
                )
            } else if self.starts_with("[") {
                (1, NodeShape::Rect, &[("]", NodeShape::Rect)])
            } else if self.starts_with("{{") {
                (2, NodeShape::Hexagon, &[("}}", NodeShape::Hexagon)])
            } else if self.starts_with("{") {
                (1, NodeShape::Diamond, &[("}", NodeShape::Diamond)])
            } else if self.starts_with(">") {
                return self.fail(at, unsupported("asymmetric (`>...]`)"));
            } else if self.starts_with("@{") {
                // Node metadata, read by `node`.
                return Ok(None);
            } else {
                return Ok(None);
            };
        let _ = shape;
        self.pos += open_len;
        let (label, shape) = self.label_until(at, closers)?;
        Ok(Some((shape, label)))
    }

    /// A node label up to one of `closers` (consumed): quoted or not. Returns the cleaned
    /// label and the shape of the closer that ended it.
    fn label_until(
        &mut self,
        open_at: usize,
        closers: &[(&str, NodeShape)],
    ) -> Step<(String, NodeShape)> {
        self.skip_inline_ws();
        let text_at = self.pos;
        let raw: String = if self.starts_with("\"`") {
            return self.fail(
                text_at,
                "markdown strings (`` \"`...`\" ``) are not supported",
            );
        } else if self.peek() == Some('"') {
            self.pos += 1;
            let start = self.pos;
            while let Some(c) = self.peek() {
                if c == '"' || c == '\n' {
                    break;
                }
                self.pos += 1;
            }
            if self.peek() != Some('"') {
                return self.fail(text_at, "unclosed `\"`");
            }
            let s: String = self.text[start..self.pos].iter().collect();
            self.pos += 1;
            self.skip_inline_ws();
            s
        } else {
            let start = self.pos;
            loop {
                match self.peek() {
                    None | Some('\n') => break,
                    _ if closers.iter().any(|(c, _)| self.starts_with(c)) => break,
                    _ => self.pos += 1,
                }
            }
            self.text[start..self.pos].iter().collect()
        };
        let Some(&(closer, shape)) = closers.iter().find(|(c, _)| self.starts_with(c)) else {
            let wanted: Vec<String> = closers.iter().map(|(c, _)| format!("`{c}`")).collect();
            return self.fail(
                open_at,
                format!("unclosed node shape: expected {}", wanted.join(" or ")),
            );
        };
        self.pos += closer.chars().count();
        let label = clean_label(&raw).map_err(|m| {
            self.issues.push(self.lines.issue(text_at, m));
            Skip
        })?;
        Ok((label, shape))
    }

    /// A link and its label, if any.
    fn link(&mut self) -> Step<Link> {
        let start = self.pos;
        let mut tail = EdgeEnd::None;
        match (self.peek(), self.peek_at(1)) {
            (Some('<'), _) => {
                tail = EdgeEnd::Arrow;
                self.pos += 1;
            }
            (Some('o'), Some('-' | '=' | '.')) => {
                tail = EdgeEnd::Circle;
                self.pos += 1;
            }
            (Some('x'), Some('-' | '=' | '.')) => {
                tail = EdgeEnd::Cross;
                self.pos += 1;
            }
            _ => {}
        }
        let mut link = match self.peek() {
            Some('-') if self.peek_at(1) == Some('.') => self.dotted_link(start)?,
            Some('-') => self.line_link(start, '-', EdgeStroke::Solid)?,
            Some('=') => self.line_link(start, '=', EdgeStroke::Thick)?,
            Some('~') => return self.fail(start, "invisible links (`~~~`) are not supported"),
            Some('&') => return self.fail(start, "expected a node before `&`"),
            Some(c) => {
                return self.fail(
                    start,
                    format!(
                        "expected a link such as `-->`, or the end of the statement, found `{c}`"
                    ),
                );
            }
            None => return self.fail(start, "expected a link"),
        };
        link.tail = tail;
        // `-->|label|`
        let save = self.pos;
        self.skip_inline_ws();
        if self.peek() == Some('|') {
            let at = self.pos;
            if link.label.is_some() {
                return self.fail(at, "this link already has a label");
            }
            self.pos += 1;
            let begin = self.pos;
            while self.peek().is_some_and(|c| c != '|' && c != '\n') {
                self.pos += 1;
            }
            if self.peek() != Some('|') {
                return self.fail(at, "unclosed `|`: a link label ends with `|`");
            }
            let raw: String = self.text[begin..self.pos].iter().collect();
            self.pos += 1;
            let raw = raw.trim();
            let raw = raw
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .unwrap_or(raw);
            link.label = Some(clean_label(raw).map_err(|m| {
                self.issues.push(self.lines.issue(begin, m));
                Skip
            })?);
        } else {
            self.pos = save;
        }
        Ok(link)
    }

    /// The end marker after a link's line: `>`, or `o`/`x` when not the start of an id.
    fn head(&mut self) -> Option<EdgeEnd> {
        let end = match self.peek()? {
            '>' => EdgeEnd::Arrow,
            'o' => EdgeEnd::Circle,
            'x' => EdgeEnd::Cross,
            _ => return None,
        };
        if end != EdgeEnd::Arrow && self.peek_at(1).is_some_and(is_id_char) {
            return None;
        }
        self.pos += 1;
        Some(end)
    }

    fn count(&mut self, c: char) -> u32 {
        let mut n = 0;
        while self.peek() == Some(c) {
            n += 1;
            self.pos += 1;
        }
        n
    }

    /// `-->`, `---`, `==>`, `===` and longer, or the labelled `-- text -->` / `== text ==>`.
    fn line_link(&mut self, start: usize, c: char, stroke: EdgeStroke) -> Step<Link> {
        let n = self.count(c);
        if n < 2 {
            return self.fail(start, format!("expected a link such as `{c}{c}>`"));
        }
        if let Some(head) = self.head() {
            return Ok(Link::new(stroke, head, n - 1));
        }
        if n >= 3 {
            return Ok(Link::new(stroke, EdgeEnd::None, n - 2));
        }
        // `-- text -->`: the text runs to the next run of the same character.
        let text_start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == '\n' || (ch == c && self.peek_at(1) == Some(c)) {
                break;
            }
            self.pos += 1;
        }
        if self.peek() != Some(c) {
            return self.fail(
                start,
                format!("unclosed link label: expected `{c}{c}>` or `{c}{c}{c}` after the text"),
            );
        }
        let raw: String = self.text[text_start..self.pos].iter().collect();
        let m = self.count(c);
        let (head, length) = match self.head() {
            Some(head) => (head, m - 1),
            None if m >= 3 => (EdgeEnd::None, m - 2),
            None => {
                return self.fail(
                    start,
                    format!(
                        "unclosed link label: expected `{c}{c}>` or `{c}{c}{c}` after the text"
                    ),
                );
            }
        };
        let mut link = Link::new(stroke, head, length);
        link.label = Some(self.cleaned(text_start, &raw)?);
        Ok(link)
    }

    /// `-.->`, `-.-`, `-..->` and the labelled `-. text .->`.
    fn dotted_link(&mut self, start: usize) -> Step<Link> {
        self.pos += 1; // '-'
        let dots = self.count('.');
        if self.peek() == Some('-') {
            self.pos += 1;
            let head = self.head().unwrap_or(EdgeEnd::None);
            return Ok(Link::new(EdgeStroke::Dotted, head, dots));
        }
        // `-. text .->`
        let text_start = self.pos;
        while let Some(ch) = self.peek() {
            if ch == '\n' || (ch == '.' && self.closes_dotted()) {
                break;
            }
            self.pos += 1;
        }
        if self.peek() != Some('.') {
            return self.fail(
                start,
                "unclosed link label: expected `.->` or `.-` after the text",
            );
        }
        let raw: String = self.text[text_start..self.pos].iter().collect();
        let dots = self.count('.');
        self.pos += 1; // '-', checked by closes_dotted
        let head = self.head().unwrap_or(EdgeEnd::None);
        let mut link = Link::new(EdgeStroke::Dotted, head, dots);
        link.label = Some(self.cleaned(text_start, &raw)?);
        Ok(link)
    }

    /// Whether `.`+ `-` starts here.
    fn closes_dotted(&self) -> bool {
        let mut k = 0;
        while self.peek_at(k) == Some('.') {
            k += 1;
        }
        k > 0 && self.peek_at(k) == Some('-')
    }

    fn cleaned(&mut self, at: usize, raw: &str) -> Step<String> {
        let raw = raw.trim();
        let raw = raw
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .unwrap_or(raw);
        clean_label(raw).map_err(|m| {
            self.issues.push(self.lines.issue(at, m));
            Skip
        })
    }
}

#[derive(Debug, Clone)]
struct Link {
    stroke: EdgeStroke,
    head: EdgeEnd,
    tail: EdgeEnd,
    length: u32,
    label: Option<String>,
}

impl Link {
    fn new(stroke: EdgeStroke, head: EdgeEnd, length: u32) -> Self {
        Self {
            stroke,
            head,
            tail: EdgeEnd::None,
            length: length.clamp(1, MAX_LENGTH),
            label: None,
        }
    }
}

/// Trims a label, turns `<br>` into a line break and decodes Mermaid's entities. Other HTML
/// is an error.
fn clean_label(raw: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = raw.trim();
    while let Some(i) = rest.find(['<', '#']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with('<') {
            let close = tail
                .find('>')
                .ok_or_else(|| "a `<` must start a `<br>` line break".to_owned())?;
            let tag = tail[1..close].trim().trim_end_matches('/').trim();
            if !tag.eq_ignore_ascii_case("br") {
                return Err(format!(
                    "HTML in labels is not supported (only `<br>`), found `{}`",
                    &tail[..=close]
                ));
            }
            out.push('\n');
            rest = &tail[close + 1..];
        } else {
            // `#name;` or `#123;`
            let end = tail[1..].find(';').map(|e| e + 1);
            let decoded = end.and_then(|e| {
                let name = &tail[1..e];
                match name {
                    "quot" => Some('"'),
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "nbsp" => Some(' '),
                    n => n.parse::<u32>().ok().and_then(char::from_u32),
                }
            });
            match (decoded, end) {
                (Some(c), Some(e)) => {
                    out.push(c);
                    rest = &tail[e + 1..];
                }
                _ => {
                    out.push('#');
                    rest = &tail[1..];
                }
            }
        }
    }
    out.push_str(rest);
    let lines: Vec<&str> = out.split('\n').map(str::trim).collect();
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(src: &str) -> Flowchart {
        parse(src).unwrap_or_else(|e| panic!("{src}\n{e:?}"))
    }

    fn err(src: &str) -> Vec<DiagramIssue> {
        parse(src).expect_err(src)
    }

    fn ids(f: &Flowchart) -> Vec<&str> {
        f.nodes.iter().map(|n| n.id.as_str()).collect()
    }

    fn edges(f: &Flowchart) -> Vec<(&str, &str)> {
        f.edges
            .iter()
            .map(|e| (f.nodes[e.from].id.as_str(), f.nodes[e.to].id.as_str()))
            .collect()
    }

    #[test]
    fn header_and_direction() {
        assert_eq!(ok("flowchart LR\nA").direction, Direction::LeftRight);
        assert_eq!(ok("graph TD;A-->B").direction, Direction::TopBottom);
        assert_eq!(ok("graph BT\n").direction, Direction::BottomTop);
        assert_eq!(ok("flowchart RL").direction, Direction::RightLeft);
        assert_eq!(ok("flowchart").direction, Direction::TopBottom);
        let e = err("flowchart XY\nA");
        assert_eq!((e[0].line, e[0].column), (1, 11));
        assert!(e[0].message.contains("unknown direction `XY`"));
    }

    #[test]
    fn chains_groups_and_reuse() {
        let f = ok("flowchart TD\n  A --> B --> C\n  A & B --> D & E\n");
        assert_eq!(ids(&f), ["A", "B", "C", "D", "E"]);
        assert_eq!(
            edges(&f),
            [
                ("A", "B"),
                ("B", "C"),
                ("A", "D"),
                ("A", "E"),
                ("B", "D"),
                ("B", "E")
            ]
        );
    }

    #[test]
    fn shapes_and_labels() {
        let f = ok(r#"flowchart TD
            a[rect] --> b(rounded) --> c([stadium]) --> d[(db)] --> e((circle))
            f{decide} --> g{{hex}} --> h[/lean/] --> i[\back\] --> j[/trap\] --> k[\alt/]
            l["quoted [with] (brackets)"]
            m[one<br>two<br/>three] --> n[#quot;hi#quot; #35;1]
        "#);
        let shapes: Vec<NodeShape> = f.nodes.iter().map(|n| n.shape).collect();
        use NodeShape::*;
        assert_eq!(
            shapes,
            [
                Rect,
                Rounded,
                Stadium,
                Cylinder,
                Circle,
                Diamond,
                Hexagon,
                Parallelogram,
                ParallelogramAlt,
                Trapezoid,
                TrapezoidAlt,
                Rect,
                Rect,
                Rect
            ]
        );
        assert_eq!(f.nodes[3].label, "db");
        assert_eq!(f.nodes[11].label, "quoted [with] (brackets)");
        assert_eq!(f.nodes[12].label, "one\ntwo\nthree");
        assert_eq!(f.nodes[13].label, "\"hi\" #1");
    }

    #[test]
    fn a_node_keeps_its_last_shape_and_its_id_as_default_label() {
        let f = ok("graph TD\nA --> B\nB[Bee] --> C\nA((Ay))");
        assert_eq!(f.nodes[0].label, "Ay");
        assert_eq!(f.nodes[0].shape, NodeShape::Circle);
        assert_eq!(f.nodes[1].label, "Bee");
        assert_eq!(f.nodes[2].label, "C");
    }

    #[test]
    fn link_kinds() {
        let f = ok("graph LR
            A --> B
            A --- C
            A -.-> D
            A -.- E
            A ==> F
            A === G
            A --o H
            A --x I
            A <--> J
            A o--o K
            A x--x L
            A ---> M
            A -..-> N
            A ====> O
            A ---- P");
        let got: Vec<(EdgeStroke, EdgeEnd, EdgeEnd, u32)> = f
            .edges
            .iter()
            .map(|e| (e.stroke, e.tail, e.head, e.length))
            .collect();
        use EdgeEnd::{Arrow, Circle, Cross, None as N};
        use EdgeStroke::*;
        assert_eq!(
            got,
            [
                (Solid, N, Arrow, 1),
                (Solid, N, N, 1),
                (Dotted, N, Arrow, 1),
                (Dotted, N, N, 1),
                (Thick, N, Arrow, 1),
                (Thick, N, N, 1),
                (Solid, N, Circle, 1),
                (Solid, N, Cross, 1),
                (Solid, Arrow, Arrow, 1),
                (Solid, Circle, Circle, 1),
                (Solid, Cross, Cross, 1),
                (Solid, N, Arrow, 2),
                (Dotted, N, Arrow, 2),
                (Thick, N, Arrow, 3),
                (Solid, N, N, 2),
            ]
        );
    }

    #[test]
    fn link_labels() {
        let f = ok("graph TD
            A -->|yes| B
            A -- no way --> C
            A -. maybe .-> D
            A == sure ==> E
            A --- |\"quoted\"| F
            A-->|a<br>b|G");
        let labels: Vec<Option<&str>> = f.edges.iter().map(|e| e.label.as_deref()).collect();
        assert_eq!(
            labels,
            [
                Some("yes"),
                Some("no way"),
                Some("maybe"),
                Some("sure"),
                Some("quoted"),
                Some("a\nb")
            ]
        );
        assert_eq!(f.edges[2].stroke, EdgeStroke::Dotted);
        assert_eq!(f.edges[3].stroke, EdgeStroke::Thick);
    }

    #[test]
    fn x_and_o_start_ids_after_a_link() {
        let f = ok("graph TD\nA-->xray\nB --> ox");
        assert_eq!(ids(&f), ["A", "xray", "B", "ox"]);
        assert_eq!(f.edges[0].head, EdgeEnd::Arrow);
    }

    #[test]
    fn comments_and_separators() {
        let f = ok("%% a comment\n\ngraph TD\n%% another\nA-->B; B-->C %% trailing\n");
        assert_eq!(edges(&f), [("A", "B"), ("B", "C")]);
    }

    #[test]
    fn unsupported_features_are_errors_with_positions() {
        let e = err("flowchart TD\n  subgraph one\n    A --> B\n  end\n");
        assert_eq!(e.len(), 2);
        assert_eq!((e[0].line, e[0].column), (2, 3));
        assert!(e[0].message.contains("subgraphs are not supported"));
        assert_eq!(e[0].excerpt, "  subgraph one");
        assert_eq!((e[1].line, e[1].column), (4, 3));

        for (src, needle) in [
            ("graph TD\nA[[sub]]", "subroutine"),
            ("graph TD\nA>flag]", "asymmetric"),
            ("graph TD\nA(((x)))", "double circle"),
            (
                "graph TD\nA@{ shape: rect }",
                "`shape` is not supported in `@{ }`",
            ),
            (
                "graph TD\nA@{ icon: \"unicorn\" }",
                "no built-in icon `unicorn`",
            ),
            ("graph TD\nA@{ icon: \"bulb\"", "unclosed `@{`"),
            ("graph TD\nA:::warn --> B", "no `classDef` named `warn`"),
            ("graph TD\nA --> B\nlinkStyle 0 stroke:#f00", "linkStyle"),
            (
                "graph TD\nA\nstyle A stroke-dasharray: 5 5",
                "stroke-dasharray",
            ),
            ("graph TD\nA\nstyle B fill:#f00", "no node `B`"),
            ("graph TD\nA ~~~ B", "invisible"),
            ("graph TD\nclick A call()", "click"),
            ("graph TD\nA[\"`**md**`\"]", "markdown"),
            ("graph TD\nA[a <b>bold</b>]", "HTML"),
            ("%%{init: {}}%%\ngraph TD\nA", "directives"),
            ("---\ntitle: T\n---\ngraph TD\nA", "front matter"),
        ] {
            let e = err(src);
            assert!(e.iter().any(|i| i.message.contains(needle)), "{src}: {e:?}");
        }
    }

    #[test]
    fn syntax_errors_name_the_place_and_parsing_goes_on() {
        let e = err("graph TD\nA[unclosed --> B\nC --> \nD -> E\nF --> G");
        let at: Vec<(u32, u32)> = e.iter().map(|i| (i.line, i.column)).collect();
        assert_eq!(at, [(2, 2), (3, 7), (4, 3)]);
        assert!(e[0].message.contains("unclosed node shape"));
        assert!(e[1].message.contains("expected a node after the link"));
        assert!(e[2].message.contains("link"));
    }

    #[test]
    fn classes_and_styles() {
        let f = ok("flowchart TD
            classDef default fill:#eee,color:#111
            classDef hot,urgent fill:#f96,stroke:#333,stroke-width:2px
            A:::hot --> B --> C
            class B,C urgent
            classDef urgent color:white
            style C fill:red");
        let st = |i: usize| &f.nodes[i].style;
        assert_eq!(st(0).fill.as_deref(), Some("#ff9966"));
        assert_eq!(st(0).color.as_deref(), Some("#111111"));
        assert_eq!(st(0).stroke_width, Some(2.0));
        // `urgent` was redefined later: the last definition is the one used.
        assert_eq!(st(1).fill.as_deref(), Some("#eeeeee"));
        assert_eq!(st(1).color.as_deref(), Some("#ffffff"));
        // `style` wins over classes.
        assert_eq!(st(2).fill.as_deref(), Some("#ff0000"));
        // A node named `class` or `style` is still a node.
        let f = ok("graph TD\nclass --> style");
        assert_eq!(ids(&f), ["class", "style"]);
    }

    #[test]
    fn style_problems_point_at_the_property() {
        let e = err("graph TD\nA\nclassDef x fill:#f96, font-size:20px");
        assert_eq!((e[0].line, e[0].column), (3, 23));
        assert!(e[0].message.contains("`font-size` is not supported"));
    }

    #[test]
    fn icons_from_node_metadata() {
        let f = ok(
            "flowchart LR\n  a@{ icon: \"bulb\" } --> b([Ship])@{icon: fa:rocket} --> c\n  c@{ icon: \"gear\", }",
        );
        let icons: Vec<Option<&str>> = f.nodes.iter().map(|n| n.icon.as_deref()).collect();
        assert_eq!(icons, [Some("bulb"), Some("fa:rocket"), Some("gear")]);
        assert_eq!(f.nodes[1].label, "Ship");
        assert_eq!(f.nodes[1].shape, NodeShape::Stadium);
    }

    #[test]
    fn the_users_own_icons_may_be_named_too() {
        let source = "flowchart LR\n  a@{ icon: \"pump\" } --> b@{ icon: \"star\" }";
        let f = parse_with_icons(source, &["pump"]).unwrap();
        assert_eq!(f.nodes[0].icon.as_deref(), Some("pump"));
        let issues = parse(source).unwrap_err();
        assert!(
            issues[0]
                .message
                .starts_with("no built-in icon `pump`; the built-in icons are")
        );
        let issues = parse_with_icons(source, &["valve", "tank"]).unwrap_err();
        assert!(
            issues[0]
                .message
                .starts_with("no built-in icon `pump` (nor among the icons given: valve, tank)"),
            "{}",
            issues[0].message
        );
    }

    #[test]
    fn diagram_kinds() {
        assert_eq!(kind("flowchart TD").unwrap(), Kind::Flowchart);
        assert_eq!(kind("  %% hi\ngraph").unwrap(), Kind::Flowchart);
        assert_eq!(kind("sequenceDiagram\n").unwrap(), Kind::Sequence);
        assert_eq!(kind("architecture-beta").unwrap(), Kind::Architecture);
        let e = kind("gantt\n").unwrap_err();
        assert!(e.message.contains("`gantt` diagrams are not supported"));
        assert!(kind("").unwrap_err().message.contains("empty"));
        assert!(
            kind("hello")
                .unwrap_err()
                .message
                .contains("expected a diagram type")
        );
    }
}

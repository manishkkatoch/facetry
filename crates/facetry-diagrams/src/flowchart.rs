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
//! * subgraphs: `subgraph id [Title]` (or `subgraph id`, `subgraph "Title"`) ... `end`, nested
//!   to any depth, with `direction TB|BT|LR|RL` inside them (read, but the diagram is laid out
//!   in its own direction), and edges to or from a subgraph by its id. A node belongs to the
//!   innermost subgraph that mentions it (the first to close, as in Mermaid);
//! * icons: `a@{ icon: "bulb" }` (alone or after a shape: `a([Idea])@{ icon: "fa:lightbulb" }`),
//!   one of Facetry's built-in icons or of the user's own ([`parse_with_icons`]);
//! * styling: `classDef name,other fill:#f96,stroke:#333,stroke-width:2px,color:#fff` (and
//!   `classDef default ...` for every node), `class a,b name`, `a:::name`, and
//!   `style a fill:#f96`; later styles win over earlier ones, `style` over classes;
//! * the theme: `%%{init: {"theme": "dark"}}%%`, or front matter with `config:` and `theme:`.
//!
//! Everything else that is valid Mermaid (`linkStyle`, other style properties,
//! click handlers, keys of `@{ }` other than `icon`, anything in a directive or front matter but
//! the theme, ...) is an
//! error that names the feature, so a diagram is never drawn with parts silently missing. All problems are
//! reported, not only the first: after an error the parser skips to the next statement.

use facetry_core::spec::{DiagramTheme, EdgeAnimation, EdgeEnd, EdgeStroke, NodeShape};

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

impl Direction {
    /// `TB`, `TD`, `BT`, `LR` or `RL`.
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "TB" | "TD" => Direction::TopBottom,
            "BT" => Direction::BottomTop,
            "LR" => Direction::LeftRight,
            "RL" => Direction::RightLeft,
            _ => return None,
        })
    }
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
    /// One of the user's blocks, from `@{ shape: "..." }` naming one.
    pub block: Option<String>,
    /// The innermost subgraph it belongs to (an index into [`Flowchart::subgraphs`]).
    pub group: Option<usize>,
}

/// One end of an edge: a node, or a whole subgraph (`a --> sub`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    /// An index into [`Flowchart::nodes`].
    Node(usize),
    /// An index into [`Flowchart::subgraphs`].
    Group(usize),
}

/// A `subgraph ... end` block.
#[derive(Debug, Clone, PartialEq)]
pub struct Subgraph {
    pub id: String,
    /// Lines separated by `\n`; the id when no title is given.
    pub title: String,
    /// The subgraph it is nested in.
    pub parent: Option<usize>,
    /// A `direction` statement inside it. Read but not used: the diagram is laid out in its
    /// own direction.
    pub direction: Option<Direction>,
    /// What `classDef`, `class` and `style` made of it (fill, stroke, title colour).
    pub style: NodeStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: End,
    pub to: End,
    pub label: Option<String>,
    pub stroke: EdgeStroke,
    pub head: EdgeEnd,
    pub tail: EdgeEnd,
    /// How many ranks the edge spans at least: 1, more for `--->` and friends.
    pub length: u32,
    /// Its id, `e1` in `a e1@--> b`, for `e1@{ ... }`.
    pub id: Option<String>,
    /// From `e1@{ animate: true }` or `animation: fast|slow`: moving or explicitly still
    /// (`animate: false`); `None` where the text does not say.
    pub animation: Option<Option<EdgeAnimation>>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Flowchart {
    pub direction: Direction,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// In the order they close: a nested subgraph comes before the one around it.
    pub subgraphs: Vec<Subgraph>,
    /// The theme the text asks for (`%%{init: {"theme": "dark"}}%%`, or `config: theme:` in
    /// front matter).
    pub theme: Option<DiagramTheme>,
}

impl Flowchart {
    /// Whether subgraph `inner` is `outer` or nested in it, at any depth.
    pub fn within(&self, inner: usize, outer: usize) -> bool {
        let mut g = Some(inner);
        while let Some(i) = g {
            if i == outer {
                return true;
            }
            g = self.subgraphs[i].parent;
        }
        false
    }

    /// How deep subgraph `g` is nested: 1 for a subgraph at the top level.
    pub fn depth(&self, g: usize) -> usize {
        let mut depth = 0;
        let mut at = Some(g);
        while let Some(i) = at {
            depth += 1;
            at = self.subgraphs[i].parent;
        }
        depth
    }
}

/// A subgraph being read: where it starts, its index once it closes, and the nodes and
/// subgraph ids mentioned in it so far.
#[derive(Debug)]
struct Open {
    at: usize,
    id: String,
    title: String,
    direction: Option<Direction>,
    nodes: Vec<usize>,
    /// Subgraphs that closed inside it (indices into `subgraphs`).
    children: Vec<usize>,
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
    /// Names of the user's blocks, which `@{ shape }` may use besides Mermaid's shapes.
    blocks: &'a [&'a str],
    pos: usize,
    issues: Vec<DiagramIssue>,
    chart: Flowchart,
    styling: Styling,
    /// Subgraphs opened and not yet closed, innermost last.
    open: Vec<Open>,
    /// Where each closed subgraph starts, for problems found once the text is read.
    subgraph_at: Vec<usize>,
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
    parse_with(source, icons, &[])
}

/// [`parse`], where `@{ icon }` may also name one of `icons` (the user's own icons) and
/// `@{ shape }` one of `blocks` (the user's own blocks).
pub fn parse_with(
    source: &str,
    icons: &[&str],
    blocks: &[&str],
) -> Result<Flowchart, Vec<DiagramIssue>> {
    let text: Vec<char> = source.chars().collect();
    let lines = Lines::new(&text);
    let mut p = Parser {
        text: &text,
        lines: &lines,
        icons,
        blocks,
        pos: 0,
        issues: Vec::new(),
        chart: Flowchart::default(),
        styling: Styling::default(),
        open: Vec::new(),
        subgraph_at: Vec::new(),
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
        blocks: &[],
        pos: 0,
        issues: Vec::new(),
        chart: Flowchart::default(),
        styling: Styling::default(),
        open: Vec::new(),
        subgraph_at: Vec::new(),
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

    /// Blank lines, `%%` comments, directives and front matter (of which only the theme is
    /// read; anything else in them is an error).
    fn skip_preamble(&mut self) {
        loop {
            self.skip_inline_ws();
            match self.peek() {
                Some('\n') => self.pos += 1,
                Some('%') if self.starts_with("%%{") => self.directive(),
                Some('%') if self.starts_with("%%") => self.skip_line(),
                Some('-') if self.starts_with("---") => self.front_matter(),
                _ => return,
            }
        }
    }

    /// `%%{init: {"theme": "dark"}}%%` (`initialize` too, either quotes, keys bare or quoted,
    /// over one line or several): the theme.
    fn directive(&mut self) {
        let start = self.pos;
        let close =
            (start + 3..self.text.len()).find(|&i| self.text[i..].starts_with(&['}', '%', '%']));
        let Some(close) = close else {
            let _: Step<()> = self.fail(start, "directive not closed with `}%%`");
            self.skip_line();
            return;
        };
        self.pos = start + 3;
        let found = self.issues.len();
        if self.init_object(close).is_err() && self.issues.len() == found {
            let _: Step<()> = self.fail(
                start,
                "could not read the directive; expected `%%{init: {\"theme\": \"dark\"}}%%`",
            );
        }
        self.pos = close + 3;
        self.skip_line();
    }

    /// The `init: { ... }` of a directive, ending before `close`.
    fn init_object(&mut self, close: usize) -> Step<()> {
        self.skip_ws_before(close);
        let at = self.pos;
        let name = self.word();
        if name != "init" && name != "initialize" {
            return self.fail(at, "only `init` directives are supported");
        }
        self.skip_ws_before(close);
        self.expect_char(':')?;
        self.skip_ws_before(close);
        self.expect_char('{')?;
        loop {
            self.skip_ws_before(close);
            match self.peek() {
                Some('}') => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(',') => self.pos += 1,
                _ if self.pos >= close => return Err(Skip),
                _ => {
                    let key_at = self.pos;
                    let key = self.directive_word()?;
                    self.skip_ws_before(close);
                    self.expect_char(':')?;
                    self.skip_ws_before(close);
                    let value_at = self.pos;
                    if self.peek() == Some('{') {
                        self.skip_braces(close)?;
                    } else {
                        let value = self.directive_word()?;
                        if key == "theme" {
                            self.set_theme(&value, value_at);
                            continue;
                        }
                    }
                    let _: Step<()> = self.fail(
                        key_at,
                        format!("`{key}` is not supported in a directive; only `theme` is"),
                    );
                }
            }
        }
    }

    /// A key or value in a directive: quoted (either quotes) or bare.
    fn directive_word(&mut self) -> Step<String> {
        match self.peek() {
            Some(q @ ('"' | '\'')) => {
                let start = self.pos + 1;
                let end = (start..self.text.len())
                    .find(|&i| self.text[i] == q || self.text[i] == '\n')
                    .filter(|&i| self.text[i] == q)
                    .ok_or(Skip)?;
                self.pos = end + 1;
                Ok(self.text[start..end].iter().collect())
            }
            _ => {
                let w = self.word();
                if w.is_empty() { Err(Skip) } else { Ok(w) }
            }
        }
    }

    /// Past a `{ ... }` with what it holds, nested ones included, ending before `close`.
    fn skip_braces(&mut self, close: usize) -> Step<()> {
        let mut depth = 0;
        while self.pos < close {
            match self.text[self.pos] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos += 1;
                        return Ok(());
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
        Err(Skip)
    }

    fn skip_ws_before(&mut self, end: usize) {
        while self.pos < end && self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn expect_char(&mut self, c: char) -> Step<()> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(Skip)
        }
    }

    fn set_theme(&mut self, name: &str, at: usize) {
        let theme = match name {
            "default" | "base" => DiagramTheme::Default,
            "neutral" => DiagramTheme::Neutral,
            "dark" => DiagramTheme::Dark,
            "forest" => DiagramTheme::Forest,
            _ => {
                let _: Step<()> = self.fail(
                    at,
                    format!(
                        "unknown theme `{name}`; expected `default`, `neutral`, `dark` or `forest`"
                    ),
                );
                return;
            }
        };
        self.chart.theme = Some(theme);
    }

    /// Front matter between `---` lines: `config:` with `theme:` under it.
    ///
    /// ```text
    /// ---
    /// config:
    ///   theme: forest
    /// ---
    /// ```
    fn front_matter(&mut self) {
        self.skip_line();
        let mut in_config = false;
        loop {
            if self.peek().is_none() {
                let at = self.pos;
                let _: Step<()> = self.fail(at, "front matter not closed with `---`");
                return;
            }
            self.pos += 1; // the line break
            let indented = matches!(self.peek(), Some(' ' | '\t'));
            self.skip_inline_ws();
            if self.starts_with("---") && !indented {
                self.skip_line();
                return;
            }
            if matches!(self.peek(), Some('\n' | '#') | None) {
                self.skip_line();
                continue;
            }
            let at = self.pos;
            let key = self.word();
            self.skip_inline_ws();
            if key.is_empty() || self.peek() != Some(':') {
                let _: Step<()> = self.fail(at, "expected `key: value` in front matter");
                self.skip_line();
                continue;
            }
            self.pos += 1;
            self.skip_inline_ws();
            let value_at = self.pos;
            let value = match self.peek() {
                Some('"' | '\'') => self.directive_word().unwrap_or_default(),
                _ => {
                    let start = self.pos;
                    self.skip_line();
                    self.text[start..self.pos]
                        .iter()
                        .collect::<String>()
                        .trim()
                        .to_owned()
                }
            };
            match (indented && in_config, key.as_str()) {
                (false, "config") if value.is_empty() => in_config = true,
                (true, "theme") => self.set_theme(&value, value_at),
                (true, _) => {
                    let _: Step<()> = self.fail(
                        at,
                        format!("`config.{key}` is not supported; only `theme` is"),
                    );
                }
                (false, _) => {
                    in_config = false;
                    let _: Step<()> = self.fail(
                        at,
                        format!(
                            "`{key}` in front matter is not supported; only `config: theme` is"
                        ),
                    );
                }
            }
            self.skip_line();
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
        self.chart.direction = match (dir.as_str(), Direction::from_name(&dir)) {
            ("", _) => Direction::TopBottom,
            (_, Some(d)) => d,
            (other, None) => {
                let _: Step<()> = self.fail(
                    at,
                    format!("unknown direction `{other}`: expected `TB`, `TD`, `BT`, `LR` or `RL`"),
                );
                self.skip_line();
                Direction::TopBottom
            }
        };
        self.statements();
        for open in std::mem::take(&mut self.open).into_iter().rev() {
            let _: Step<()> = self.fail(
                open.at,
                format!("subgraph `{}` is never closed (`end` is missing)", open.id),
            );
        }
        self.resolve_group_ends();
        self.resolve_styles();
    }

    /// A node named like a subgraph is that subgraph, as an edge's end (`a --> sub`): the
    /// node goes and its edges end at the subgraph.
    fn resolve_group_ends(&mut self) {
        let mut remove = Vec::new();
        for (g, sub) in self.chart.subgraphs.iter().enumerate() {
            if let Some(i) = self.chart.nodes.iter().position(|n| n.id == sub.id) {
                remove.push((i, g));
            }
        }
        for &(i, g) in &remove {
            let node = &self.chart.nodes[i];
            if node.shape != NodeShape::Rect || node.label != node.id || node.icon.is_some() {
                let at = self.subgraph_at[g];
                let id = self.chart.subgraphs[g].id.clone();
                let _: Step<()> = self.fail(
                    at,
                    format!("`{id}` is both a node (with a shape) and a subgraph"),
                );
            }
        }
        if remove.is_empty() {
            return;
        }
        let gone = |i: usize| remove.iter().find(|(n, _)| *n == i).map(|&(_, g)| g);
        let shift = |i: usize| i - remove.iter().filter(|(n, _)| *n < i).count();
        let map = |end: End| match end {
            End::Node(i) => match gone(i) {
                Some(g) => End::Group(g),
                None => End::Node(shift(i)),
            },
            group => group,
        };
        for e in &mut self.chart.edges {
            e.from = map(e.from);
            e.to = map(e.to);
        }
        let mut i = 0;
        self.chart.nodes.retain(|_| {
            i += 1;
            gone(i - 1).is_none()
        });
        // An edge between a subgraph and something inside it has no way to run.
        let chart = &self.chart;
        let inside = |end: End, g: usize| match end {
            End::Node(n) => chart.nodes[n].group.is_some_and(|h| chart.within(h, g)),
            End::Group(h) => chart.within(h, g),
        };
        let mut problems = Vec::new();
        for e in &chart.edges {
            for (a, b) in [(e.from, e.to), (e.to, e.from)] {
                if let End::Group(g) = a {
                    if inside(b, g) {
                        problems.push(g);
                    }
                }
            }
        }
        for g in problems {
            let (at, id) = (self.subgraph_at[g], self.chart.subgraphs[g].id.clone());
            let _: Step<()> = self.fail(
                at,
                format!("an edge joins subgraph `{id}` to something inside it"),
            );
        }
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
            self.apply_style(node, style, *at);
        }
        for (node, style, at) in &styling.styles {
            self.apply_style(node, style, *at);
        }
    }

    /// Styles node or subgraph `id`.
    fn apply_style(&mut self, id: &str, style: &NodeStyle, at: usize) {
        if let Some(n) = self.chart.nodes.iter_mut().find(|n| n.id == id) {
            n.style.apply(style);
        } else if let Some(g) = self.chart.subgraphs.iter_mut().find(|g| g.id == id) {
            g.style.apply(style);
        } else {
            let _: Step<()> = self.fail(at, format!("no node or subgraph `{id}` to style"));
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
        let alone = matches!(next, None | Some(' ' | '\t' | '\r' | '\n' | ';'));
        if alone {
            match word.as_str() {
                "subgraph" => {
                    self.pos = end;
                    let opened = self.open.len();
                    let result = self.subgraph_start(start);
                    if result.is_err() && self.open.len() == opened {
                        // Already reported: something for its `end` to close.
                        self.open.push(Open {
                            at: start,
                            id: String::new(),
                            title: String::new(),
                            direction: None,
                            nodes: Vec::new(),
                            children: Vec::new(),
                        });
                    }
                    return result;
                }
                "end" => {
                    self.pos = end;
                    return self.subgraph_end(start);
                }
                "direction" => {
                    self.pos = end;
                    return self.direction_statement(start);
                }
                _ => {}
            }
        }
        // `e1@{ animate: true }`: an edge's metadata, by the id its link gave it.
        if self.text[end..].starts_with(&['@', '{']) {
            if let Some(edge) = self
                .chart
                .edges
                .iter()
                .position(|e| e.id.as_deref() == Some(&word))
            {
                self.pos = end;
                return self.edge_metadata(edge);
            }
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
            let id = self.edge_id()?;
            let link = self.link()?;
            self.skip_inline_ws();
            let next = self.node_group()?;
            if let Some((_, at)) = &id {
                if group.len() * next.len() > 1 {
                    return self.fail(*at, "an edge id names one edge; `&` here makes several");
                }
            }
            for &a in &group {
                for &b in &next {
                    self.chart.edges.push(Edge {
                        from: End::Node(a),
                        to: End::Node(b),
                        label: link.label.clone(),
                        stroke: link.stroke,
                        head: link.head,
                        tail: link.tail,
                        length: link.length,
                        id: id.as_ref().map(|(id, _)| id.clone()),
                        animation: None,
                    });
                }
            }
            group = next;
        }
    }

    /// An edge's id in front of its link, `e1@` in `a e1@--> b`, and where it is.
    fn edge_id(&mut self) -> Step<Option<(String, usize)>> {
        let at = self.pos;
        let mut end = at;
        while self.text.get(end).copied().is_some_and(is_id_char) {
            end += 1;
        }
        if end == at || self.text.get(end) != Some(&'@') || self.text.get(end + 1) == Some(&'{') {
            return Ok(None);
        }
        let id: String = self.text[at..end].iter().collect();
        if self
            .chart
            .edges
            .iter()
            .any(|e| e.id.as_deref() == Some(&id))
        {
            return self.fail(at, format!("edge id `{id}` is used twice"));
        }
        self.pos = end + 1;
        Ok(Some((id, at)))
    }

    /// `@{ key: value, ... }` (the `@{` next): the pairs, each with where its key and its
    /// value are; a value quoted or bare.
    fn metadata_pairs(&mut self) -> Step<Vec<(String, usize, String, usize)>> {
        let open = self.pos;
        self.pos += 2;
        let mut out = Vec::new();
        loop {
            self.skip_inline_ws();
            match self.peek() {
                Some('}') => {
                    self.pos += 1;
                    return Ok(out);
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
            out.push((key, key_at, value, value_at));
        }
    }

    /// `e1@{ animate: true }` (the `@{` next) for edge `edge`: `animate` (`true` or `false`)
    /// and `animation` (`fast` or `slow`, which animates it too).
    fn edge_metadata(&mut self, edge: usize) -> Step<()> {
        for (key, key_at, value, value_at) in self.metadata_pairs()? {
            let animation = match (key.as_str(), value.as_str()) {
                ("animate", "true") => Some(EdgeAnimation::Fast),
                ("animate", "false") => None,
                ("animation", "fast") => Some(EdgeAnimation::Fast),
                ("animation", "slow") => Some(EdgeAnimation::Slow),
                ("animate", _) => {
                    return self.fail(value_at, "`animate` is `true` or `false`");
                }
                ("animation", _) => {
                    return self.fail(value_at, "`animation` is `fast` or `slow`");
                }
                _ => {
                    return self.fail(
                        key_at,
                        format!(
                            "`{key}` is not supported in an edge's `@{{ }}` (only `animate` and `animation`)"
                        ),
                    );
                }
            };
            // `animate: true` keeps a pace set by `animation`.
            let e = &mut self.chart.edges[edge];
            e.animation = match (key.as_str(), animation, e.animation) {
                ("animate", Some(_), Some(Some(pace))) => Some(Some(pace)),
                _ => Some(animation),
            };
        }
        Ok(())
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
        let meta = if self.starts_with("@{") {
            self.metadata()?
        } else {
            Meta::default()
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
                    block: None,
                    group: None,
                });
                self.chart.nodes.len() - 1
            }
        };
        if let Some(open) = self.open.last_mut() {
            open.nodes.push(index);
        }
        if meta.icon.is_some() {
            self.chart.nodes[index].icon = meta.icon;
        }
        if let Some((shape, label)) = shape {
            let node = &mut self.chart.nodes[index];
            node.shape = shape;
            node.label = label;
        }
        let node = &mut self.chart.nodes[index];
        if let Some(shape) = meta.shape {
            node.shape = shape;
            node.block = None;
        }
        if meta.block.is_some() {
            node.block = meta.block;
        }
        if let Some(label) = meta.label {
            node.label = label;
        }
        Ok(index)
    }

    /// Node metadata, `@{ icon: "bulb", shape: cyl, label: "Orders" }`: of Mermaid's keys
    /// `icon` (one of the built-in icons or of the user's, `fa:`-style prefixes dropped for the
    /// built-in ones), `shape` (one of Mermaid's shape names, or a block of the user's) and
    /// `label` are read.
    fn metadata(&mut self) -> Step<Meta> {
        let mut meta = Meta::default();
        for (key, key_at, value, value_at) in self.metadata_pairs()? {
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
                    meta.icon = Some(value);
                }
                "shape" => {
                    if self.blocks.contains(&value.as_str()) {
                        meta.block = Some(value);
                        meta.shape = None;
                        continue;
                    }
                    match mermaid_shape(&value) {
                        Ok(shape) => {
                            meta.shape = Some(shape);
                            meta.block = None;
                        }
                        Err(why) => {
                            let blocks = if self.blocks.is_empty() {
                                String::new()
                            } else {
                                format!("; the blocks given are {}", self.blocks.join(", "))
                            };
                            return self.fail(value_at, format!("{why}{blocks}"));
                        }
                    }
                }
                "label" => {
                    meta.label = Some(self.cleaned(value_at, &value)?);
                }
                "animate" | "animation" => {
                    return self.fail(
                        key_at,
                        format!(
                            "`{key}` is for edges: give the edge an id (`a e1@--> b`), then write `e1@{{ {key}: ... }}`"
                        ),
                    );
                }
                _ => {
                    return self.fail(
                        key_at,
                        format!(
                            "`{key}` is not supported in `@{{ }}` (only `icon`, `shape` and `label`)"
                        ),
                    );
                }
            }
        }
        Ok(meta)
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

    /// `subgraph id`, `subgraph id [Title]` (or `id["Title"]`), `subgraph "Title"`, or
    /// `subgraph Some title` (the words are both id and title); after the keyword.
    fn subgraph_start(&mut self, keyword: usize) -> Step<()> {
        self.skip_inline_ws();
        let at = self.pos;
        let (id, title) = if self.peek() == Some('"') {
            let start = self.pos;
            self.pos += 1;
            while self.peek().is_some_and(|c| c != '"' && c != '\n') {
                self.pos += 1;
            }
            if self.peek() != Some('"') {
                return self.fail(start, "unclosed `\"`");
            }
            self.pos += 1;
            let raw: String = self.text[start..self.pos].iter().collect();
            let title = self.cleaned(start, &raw)?;
            (title.replace('\n', " "), title)
        } else {
            while self.peek().is_some_and(|c| is_id_char(c) || c == '-') {
                self.pos += 1;
            }
            let id: String = self.text[at..self.pos].iter().collect();
            if id.is_empty() {
                return self.fail(at, "expected the subgraph's id or title");
            }
            self.skip_inline_ws();
            if self.peek() == Some('[') {
                let open = self.pos;
                self.pos += 1;
                let start = self.pos;
                let mut quoted = false;
                while let Some(c) = self.peek() {
                    if c == '"' {
                        quoted = !quoted;
                    } else if (c == ']' && !quoted) || c == '\n' {
                        break;
                    }
                    self.pos += 1;
                }
                if self.peek() != Some(']') {
                    return self.fail(open, "unclosed `[` in the subgraph's title");
                }
                let raw: String = self.text[start..self.pos].iter().collect();
                self.pos += 1;
                let title = self.cleaned(start, &raw)?;
                (id, title)
            } else if self.at_statement_end() {
                (id.clone(), id)
            } else {
                // Words: the whole line names it.
                while !self.at_statement_end() {
                    self.pos += 1;
                }
                let raw: String = self.text[at..self.pos].iter().collect();
                let title = self.cleaned(at, &raw)?;
                (title.clone(), title)
            }
        };
        self.skip_inline_ws();
        if !self.at_statement_end() {
            let at = self.pos;
            return self.fail(at, "unexpected text after the subgraph's title");
        }
        let taken =
            self.chart.subgraphs.iter().any(|g| g.id == id) || self.open.iter().any(|o| o.id == id);
        if taken {
            // Reported, but opened all the same so that its `end` has something to close.
            let _: Step<()> = self.fail(at, format!("a second subgraph called `{id}`"));
        }
        self.open.push(Open {
            at: keyword,
            id,
            title,
            direction: None,
            nodes: Vec::new(),
            children: Vec::new(),
        });
        Ok(())
    }

    /// `end`: closes the innermost open subgraph. Its nodes not already in a subgraph that
    /// closed earlier join it.
    fn subgraph_end(&mut self, at: usize) -> Step<()> {
        if !self.at_statement_end() {
            let here = self.pos;
            return self.fail(here, "unexpected text after `end`");
        }
        let Some(open) = self.open.pop() else {
            return self.fail(at, "`end` without a `subgraph` to close");
        };
        let g = self.chart.subgraphs.len();
        for &n in &open.nodes {
            let node = &mut self.chart.nodes[n];
            if node.group.is_none() {
                node.group = Some(g);
            }
        }
        for &child in &open.children {
            self.chart.subgraphs[child].parent = Some(g);
        }
        if let Some(outer) = self.open.last_mut() {
            outer.children.push(g);
            // The outer subgraph holds what this one does too, for nodes still unplaced.
            outer.nodes.extend(open.nodes.iter().copied());
        }
        self.chart.subgraphs.push(Subgraph {
            id: open.id,
            title: open.title,
            parent: None,
            direction: open.direction,
            style: NodeStyle::default(),
        });
        self.subgraph_at.push(open.at);
        Ok(())
    }

    /// `direction LR` inside a subgraph.
    fn direction_statement(&mut self, at: usize) -> Step<()> {
        self.skip_inline_ws();
        let name_at = self.pos;
        let name = self.word();
        let Some(direction) = Direction::from_name(&name) else {
            return self.fail(
                name_at,
                format!("unknown direction `{name}`: expected `TB`, `TD`, `BT`, `LR` or `RL`"),
            );
        };
        if !self.at_statement_end() {
            let here = self.pos;
            return self.fail(here, "unexpected text after the direction");
        }
        match self.open.last_mut() {
            Some(open) => {
                open.direction = Some(direction);
                Ok(())
            }
            None => self.fail(at, "`direction` belongs inside a subgraph"),
        }
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
/// What a node's `@{ }` sets.
#[derive(Default)]
struct Meta {
    icon: Option<String>,
    shape: Option<NodeShape>,
    block: Option<String>,
    label: Option<String>,
}

/// Mermaid 11's shapes this subset draws, by every name Mermaid gives them.
const MERMAID_SHAPES: [(&str, NodeShape); 33] = [
    ("rect", NodeShape::Rect),
    ("proc", NodeShape::Rect),
    ("process", NodeShape::Rect),
    ("rectangle", NodeShape::Rect),
    ("rounded", NodeShape::Rounded),
    ("event", NodeShape::Rounded),
    ("stadium", NodeShape::Stadium),
    ("pill", NodeShape::Stadium),
    ("terminal", NodeShape::Stadium),
    ("cyl", NodeShape::Cylinder),
    ("cylinder", NodeShape::Cylinder),
    ("database", NodeShape::Cylinder),
    ("db", NodeShape::Cylinder),
    ("circle", NodeShape::Circle),
    ("circ", NodeShape::Circle),
    ("diam", NodeShape::Diamond),
    ("diamond", NodeShape::Diamond),
    ("decision", NodeShape::Diamond),
    ("question", NodeShape::Diamond),
    ("hex", NodeShape::Hexagon),
    ("hexagon", NodeShape::Hexagon),
    ("prepare", NodeShape::Hexagon),
    ("lean-r", NodeShape::Parallelogram),
    ("lean-right", NodeShape::Parallelogram),
    ("in-out", NodeShape::Parallelogram),
    ("lean-l", NodeShape::ParallelogramAlt),
    ("lean-left", NodeShape::ParallelogramAlt),
    ("out-in", NodeShape::ParallelogramAlt),
    ("trap-b", NodeShape::Trapezoid),
    ("trapezoid-bottom", NodeShape::Trapezoid),
    ("priority", NodeShape::Trapezoid),
    ("trap-t", NodeShape::TrapezoidAlt),
    ("trapezoid-top", NodeShape::TrapezoidAlt),
];

/// Mermaid 11's other shape names: valid Mermaid, not drawn by this subset.
const OTHER_MERMAID_SHAPES: [&str; 33] = [
    "fr-rect",
    "subproc",
    "subprocess",
    "subroutine",
    "framed-rectangle",
    "odd",
    "asymmetric",
    "dbl-circ",
    "double-circle",
    "text",
    "notch-rect",
    "card",
    "lin-rect",
    "lined-process",
    "sm-circ",
    "small-circle",
    "fr-circ",
    "framed-circle",
    "fork",
    "join",
    "hourglass",
    "bolt",
    "doc",
    "document",
    "delay",
    "h-cyl",
    "lin-cyl",
    "curv-trap",
    "div-rect",
    "tri",
    "triangle",
    "flag",
    "docs",
];

/// The shape Mermaid's shape name `name` stands for, or why there is none.
fn mermaid_shape(name: &str) -> Result<NodeShape, String> {
    if let Some((_, shape)) = MERMAID_SHAPES.iter().find(|(n, _)| *n == name) {
        return Ok(*shape);
    }
    if name == "manual" || name == "inv-trapezoid" {
        return Ok(NodeShape::TrapezoidAlt);
    }
    if OTHER_MERMAID_SHAPES.contains(&name) {
        return Err(format!("the Mermaid shape `{name}` is not supported"));
    }
    Err(format!(
        "no shape `{name}`: the shapes are Mermaid's rect, rounded, stadium, cyl, circle, diam, \
         hex, lean-r, lean-l, trap-b and trap-t (and their other names)"
    ))
}

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
        let name = |end: End| match end {
            End::Node(i) => f.nodes[i].id.as_str(),
            End::Group(g) => f.subgraphs[g].id.as_str(),
        };
        f.edges.iter().map(|e| (name(e.from), name(e.to))).collect()
    }

    /// Each node's id and the id of the subgraph it is in.
    fn groups(f: &Flowchart) -> Vec<(&str, Option<&str>)> {
        f.nodes
            .iter()
            .map(|n| (n.id.as_str(), n.group.map(|g| f.subgraphs[g].id.as_str())))
            .collect()
    }

    #[test]
    fn subgraphs_hold_the_nodes_mentioned_in_them_innermost_first() {
        let f = ok(
            "flowchart TD\n  A --> B\n  subgraph outer [The outside]\n    B --> C\n    subgraph inner\n      direction LR\n      C --> D\n    end\n    D --> E\n  end\n  subgraph \"Second one\"\n    E --> F\n  end\n  subgraph Some words here\n    G\n  end",
        );
        assert_eq!(
            groups(&f),
            [
                ("A", None),
                ("B", Some("outer")),
                ("C", Some("inner")),
                ("D", Some("inner")),
                ("E", Some("outer")),
                ("F", Some("Second one")),
                ("G", Some("Some words here")),
            ]
        );
        let sub = |id: &str| f.subgraphs.iter().find(|g| g.id == id).unwrap();
        assert_eq!(sub("outer").title, "The outside");
        assert_eq!(sub("inner").title, "inner");
        assert_eq!(sub("inner").direction, Some(Direction::LeftRight));
        assert_eq!(sub("Second one").title, "Second one");
        // Nesting: inner closes first and sits in outer.
        let (inner, outer) = (0, 1);
        assert_eq!(f.subgraphs[inner].id, "inner");
        assert_eq!(f.subgraphs[inner].parent, Some(outer));
        assert_eq!(f.subgraphs[outer].parent, None);
        assert!(f.within(inner, outer) && !f.within(outer, inner));
        assert_eq!((f.depth(inner), f.depth(outer)), (2, 1));
    }

    #[test]
    fn edges_may_end_at_a_subgraph_and_subgraphs_take_styles() {
        let f = ok(
            "flowchart LR\n  start --> work\n  subgraph work [Work]\n    a --> b\n  end\n  work --> done\n  style work fill:#eef\n  classDef hot stroke:#f00\n  class work hot",
        );
        assert_eq!(ids(&f), ["start", "a", "b", "done"]);
        assert_eq!(edges(&f), [("start", "work"), ("a", "b"), ("work", "done")]);
        assert_eq!(f.edges[0].to, End::Group(0));
        assert_eq!(f.subgraphs[0].style.fill.as_deref(), Some("#eeeeff"));
        assert_eq!(f.subgraphs[0].style.stroke.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn subgraph_mistakes_are_errors_where_they_are() {
        let one = |src: &str| {
            let e = err(src);
            assert_eq!(e.len(), 1, "{src}: {e:?}");
            (e[0].line, e[0].column, e[0].message.clone())
        };
        let (l, c, m) = one("flowchart TD\n  subgraph one\n    A\n");
        assert_eq!((l, c), (2, 3));
        assert!(m.contains("never closed"), "{m}");
        let (l, _, m) = one("flowchart TD\n  A\n  end\n");
        assert_eq!(l, 3);
        assert!(m.contains("without a `subgraph`"), "{m}");
        assert!(
            one("flowchart TD\n  direction LR")
                .2
                .contains("inside a subgraph")
        );
        assert!(
            one("flowchart TD\nsubgraph s\n direction UP\nend")
                .2
                .contains("unknown direction")
        );
        assert!(
            one("flowchart TD\nsubgraph s\nend\nsubgraph s\nend")
                .2
                .contains("second subgraph")
        );
        assert!(
            one("flowchart TD\nsubgraph s\n a --> s\nend")
                .2
                .contains("inside it")
        );
        assert!(
            one("flowchart TD\ns[Box] --> b\nsubgraph s\n c\nend")
                .2
                .contains("both a node")
        );
        assert!(
            one("flowchart TD\nsubgraph s [unclosed\nend")
                .2
                .contains("unclosed `[`")
        );
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
        for (src, needle) in [
            ("graph TD\nA[[sub]]", "subroutine"),
            ("graph TD\nA>flag]", "asymmetric"),
            ("graph TD\nA(((x)))", "double circle"),
            (
                "graph TD\nA@{ curve: basis }",
                "`curve` is not supported in `@{ }` (only `icon`, `shape` and `label`)",
            ),
            (
                "graph TD\nA@{ shape: docs }",
                "the Mermaid shape `docs` is not supported",
            ),
            ("graph TD\nA@{ shape: pump }", "no shape `pump`"),
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
            ("graph TD\nA\nstyle B fill:#f00", "no node or subgraph `B`"),
            ("graph TD\nA ~~~ B", "invisible"),
            ("graph TD\nclick A call()", "click"),
            ("graph TD\nA[\"`**md**`\"]", "markdown"),
            ("graph TD\nA[a <b>bold</b>]", "HTML"),
            ("%%{wrap}%%\ngraph TD\nA", "only `init` directives"),
            (
                "%%{init: {\"flowchart\": {\"curve\": \"step\"}}}%%\ngraph TD\nA",
                "`flowchart` is not supported in a directive",
            ),
            (
                "%%{init: {'theme': 'pink'}}%%\ngraph TD\nA",
                "unknown theme `pink`",
            ),
            ("---\ntitle: T\n---\ngraph TD\nA", "`title` in front matter"),
            (
                "---\nconfig:\n  look: handDrawn\n---\ngraph TD\nA",
                "`config.look` is not supported",
            ),
        ] {
            let e = err(src);
            assert!(e.iter().any(|i| i.message.contains(needle)), "{src}: {e:?}");
        }
    }

    #[test]
    fn mermaid_shape_names_blocks_and_labels_in_metadata() {
        let f = ok(
            "flowchart LR\n  a@{ shape: cyl, label: \"Orders\" } --> b@{ shape: diam }\n  c[Text]@{ shape: lean-l }",
        );
        let got: Vec<_> = f
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.shape, n.label.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("a", NodeShape::Cylinder, "Orders"),
                ("b", NodeShape::Diamond, "b"),
                ("c", NodeShape::ParallelogramAlt, "Text"),
            ]
        );
        // A block of the user's, by its name; a Mermaid name given later puts the shape back.
        let f = parse_with(
            "flowchart LR\n  p[Feed pump]@{ shape: pump } --> q@{ shape: pump }\n  q@{ shape: rect }",
            &[],
            &["pump"],
        )
        .unwrap();
        assert_eq!(f.nodes[0].block.as_deref(), Some("pump"));
        assert_eq!(f.nodes[0].label, "Feed pump");
        assert_eq!(f.nodes[1].block, None);
        // An unknown one names the blocks there are.
        let e = parse_with(
            "flowchart LR\n  p@{ shape: pumpp }",
            &[],
            &["pump", "mixer"],
        )
        .unwrap_err();
        assert!(
            e[0].message.ends_with("the blocks given are pump, mixer"),
            "{e:?}"
        );
    }

    #[test]
    fn edges_take_ids_and_animate_by_them() {
        let f = ok(
            "flowchart LR\n  a e1@--> b e2@-.-> c --> d\n  e1@{ animate: true }\n  e2@{ animation: slow }",
        );
        let anim: Vec<_> = f
            .edges
            .iter()
            .map(|e| (e.id.as_deref(), e.animation))
            .collect();
        assert_eq!(
            anim,
            [
                (Some("e1"), Some(Some(EdgeAnimation::Fast))),
                (Some("e2"), Some(Some(EdgeAnimation::Slow))),
                (None, None),
            ]
        );
        assert_eq!(edges(&f), [("a", "b"), ("b", "c"), ("c", "d")]);
        // A pace given first stays; `animate: false` stills the edge.
        let f = ok("flowchart LR\n  a e1@--> b\n  e1@{ animation: slow, animate: true }");
        assert_eq!(f.edges[0].animation, Some(Some(EdgeAnimation::Slow)));
        let f = ok("flowchart LR\n  a e1@--> b\n  e1@{ animate: false }");
        assert_eq!(f.edges[0].animation, Some(None));
        // `x@{ }` names a node unless an edge took that id.
        assert_eq!(
            ids(&ok("flowchart LR\n  a --> b\n  e1@{ icon: bulb }")),
            ["a", "b", "e1"]
        );
        for (src, needle) in [
            (
                "flowchart LR\n  a e1@--> b\n  c e1@--> d",
                "edge id `e1` is used twice",
            ),
            ("flowchart LR\n  a & b e1@--> c", "makes several"),
            (
                "flowchart LR\n  a e1@--> b\n  e1@{ animate: yes }",
                "`true` or `false`",
            ),
            (
                "flowchart LR\n  a e1@--> b\n  e1@{ animation: medium }",
                "`fast` or `slow`",
            ),
            (
                "flowchart LR\n  a e1@--> b\n  e1@{ curve: linear }",
                "`curve` is not supported in an edge",
            ),
            (
                "flowchart LR\n  a@{ animate: true } --> b",
                "`animate` is for edges",
            ),
        ] {
            let e = err(src);
            assert!(e.iter().any(|i| i.message.contains(needle)), "{src}: {e:?}");
        }
    }

    #[test]
    fn a_theme_comes_from_a_directive_or_front_matter() {
        assert_eq!(ok("graph TD\nA").theme, None);
        for src in [
            "%%{init: {\"theme\": \"dark\"}}%%\ngraph TD\nA",
            "%%{ initialize: { 'theme': 'dark' } }%%\ngraph TD\nA",
            "%%{init: {theme: dark}}%%\ngraph TD\nA",
            "%%{\n  init: {\n    \"theme\": \"dark\"\n  }\n}%%\ngraph TD\nA",
            "---\nconfig:\n  theme: dark\n---\ngraph TD\nA",
            "---\n# a comment\nconfig:\n  theme: \"dark\"\n---\n%% and one here\ngraph TD\nA",
        ] {
            let chart = ok(src);
            assert_eq!(chart.theme, Some(DiagramTheme::Dark), "{src}");
            assert_eq!(chart.nodes.len(), 1, "{src}");
        }
        assert_eq!(
            ok("%%{init: {'theme':'base'}}%%\ngraph TD\nA").theme,
            Some(DiagramTheme::Default)
        );
        // An unclosed directive is reported where it starts.
        let e = err("%%{init: {'theme': 'dark'}\ngraph TD\nA");
        assert_eq!((e[0].line, e[0].column), (1, 1));
        assert!(e[0].message.contains("not closed"), "{e:?}");
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

//! Layout quality, measured: what makes a diagram hard to read, counted. The golden corpus
//! records these for every diagram, so a change to the layout that makes any of them worse
//! shows up in review.

use facetry_core::spec::DiagramSpec;

/// Quality numbers of one laid-out diagram (layout units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub nodes: usize,
    pub edges: usize,
    /// Pairs of edges that cross (each crossing point counted once).
    pub crossings: usize,
    /// Edges passing through a node they do not start or end at, or through a card.
    pub edge_node_overlaps: usize,
    /// Pairs of nodes (or their cards) whose rectangles overlap (should always be 0).
    pub node_overlaps: usize,
    /// Edge labels overlapping a node or another edge label.
    pub label_overlaps: usize,
    /// Corners in the edges' control polylines sharper than 15 degrees.
    pub bends: usize,
    /// Total length of the drawn edges.
    pub edge_length: f64,
    pub width: f64,
    pub height: f64,
}

impl Metrics {
    pub fn area(&self) -> f64 {
        self.width * self.height
    }

    pub fn aspect(&self) -> f64 {
        if self.height > 0.0 {
            self.width / self.height
        } else {
            0.0
        }
    }
}

impl std::fmt::Display for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "nodes {} edges {} | crossings {} edge-node overlaps {} node overlaps {} label overlaps {} bends {} | edge length {:.0} size {:.0} x {:.0}",
            self.nodes,
            self.edges,
            self.crossings,
            self.edge_node_overlaps,
            self.node_overlaps,
            self.label_overlaps,
            self.bends,
            self.edge_length,
            self.width,
            self.height,
        )
    }
}

type Rect = [f64; 4]; // x0 y0 x1 y1

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Proper intersection of segments ab and cd (touching at ends does not count).
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let denom = r[0] * s[1] - r[1] * s[0];
    if denom.abs() < 1e-12 {
        return None;
    }
    let q = [c[0] - a[0], c[1] - a[1]];
    let t = (q[0] * s[1] - q[1] * s[0]) / denom;
    let u = (q[0] * r[1] - q[1] * r[0]) / denom;
    const E: f64 = 1e-6;
    (t > E && t < 1.0 - E && u > E && u < 1.0 - E).then(|| [a[0] + t * r[0], a[1] + t * r[1]])
}

/// Measures `spec`; `control` are the edges' polylines before smoothing (for bends).
pub fn measure(spec: &DiagramSpec, control: &[Vec<[f64; 2]>], font: f64) -> Metrics {
    let rects: Vec<Rect> = spec
        .nodes
        .iter()
        .map(|n| {
            [
                n.x - n.width / 2.0,
                n.y - n.height / 2.0,
                n.x + n.width / 2.0,
                n.y + n.height / 2.0,
            ]
        })
        .collect();
    let index = |id: &str| spec.nodes.iter().position(|n| n.id == id);
    // Everything an edge must keep clear of, with the node it belongs to: the nodes, and their
    // cards (an edge must not cross even its own node's card).
    let mut obstacles: Vec<(usize, bool, Rect)> = rects
        .iter()
        .enumerate()
        .map(|(i, r)| (i, true, *r))
        .collect();
    for (i, n) in spec.nodes.iter().enumerate() {
        if let Some(c) = &n.card {
            let r = [
                c.x - c.width / 2.0,
                c.y - c.height / 2.0,
                c.x + c.width / 2.0,
                c.y + c.height / 2.0,
            ];
            obstacles.push((i, false, r));
        }
    }

    let mut crossings = 0;
    for i in 0..spec.edges.len() {
        for j in i + 1..spec.edges.len() {
            let (a, b) = (&spec.edges[i].points, &spec.edges[j].points);
            let mut found: Vec<[f64; 2]> = Vec::new();
            for s in a.windows(2) {
                for t in b.windows(2) {
                    if let Some(p) = cross(s[0], s[1], t[0], t[1]) {
                        // A curve sampled into segments can report one crossing twice.
                        if !found.iter().any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 2.0) {
                            found.push(p);
                        }
                    }
                }
            }
            crossings += found.len();
        }
    }

    let mut edge_node_overlaps = 0;
    for e in &spec.edges {
        let ends = [index(&e.from), index(&e.to)];
        for (k, body, r) in &obstacles {
            if *body && ends.contains(&Some(*k)) {
                continue;
            }
            let inner = [r[0] + 2.0, r[1] + 2.0, r[2] - 2.0, r[3] - 2.0];
            let hit = e
                .points
                .windows(2)
                .any(|s| segment_hits_rect(s[0], s[1], &inner));
            if hit {
                edge_node_overlaps += 1;
            }
        }
    }

    let mut node_overlaps = 0;
    for i in 0..obstacles.len() {
        for j in i + 1..obstacles.len() {
            let ((a, _, ra), (b, _, rb)) = (&obstacles[i], &obstacles[j]);
            if a != b && overlaps(ra, rb) {
                node_overlaps += 1;
            }
        }
    }

    let font_m = facetry_font::FontMetrics::inter();
    let labels: Vec<Rect> = spec
        .edges
        .iter()
        .filter_map(|e| {
            let p = e.label_position?;
            let text = e.label.as_deref()?;
            let size = font * 0.9;
            let lines: Vec<&str> = text.split('\n').collect();
            let w = lines
                .iter()
                .map(|l| f64::from(font_m.text_width(l, size as f32)))
                .fold(0.0, f64::max);
            let h = lines.len() as f64 * size * 1.25;
            Some([
                p[0] - w / 2.0,
                p[1] - h / 2.0,
                p[0] + w / 2.0,
                p[1] + h / 2.0,
            ])
        })
        .collect();
    let mut label_overlaps = 0;
    for (i, l) in labels.iter().enumerate() {
        if rects.iter().any(|r| overlaps(l, r)) || labels[..i].iter().any(|m| overlaps(l, m)) {
            label_overlaps += 1;
        }
    }

    let mut bends = 0;
    for c in control {
        for w in c.windows(3) {
            let (u, v) = (
                [w[1][0] - w[0][0], w[1][1] - w[0][1]],
                [w[2][0] - w[1][0], w[2][1] - w[1][1]],
            );
            let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
            if lu < 1e-9 || lv < 1e-9 {
                continue;
            }
            let cos = (u[0] * v[0] + u[1] * v[1]) / (lu * lv);
            if cos < 15f64.to_radians().cos() {
                bends += 1;
            }
        }
    }

    let edge_length = spec
        .edges
        .iter()
        .map(|e| {
            e.points
                .windows(2)
                .map(|s| (s[1][0] - s[0][0]).hypot(s[1][1] - s[0][1]))
                .sum::<f64>()
        })
        .sum();

    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut add = |p: [f64; 2]| {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    };
    for r in rects.iter().chain(&labels) {
        add([r[0], r[1]]);
        add([r[2], r[3]]);
    }
    spec.edges
        .iter()
        .flat_map(|e| &e.points)
        .for_each(|&p| add(p));
    let (width, height) = if lo[0].is_finite() {
        (hi[0] - lo[0], hi[1] - lo[1])
    } else {
        (0.0, 0.0)
    };
    Metrics {
        nodes: spec.nodes.len(),
        edges: spec.edges.len(),
        crossings,
        edge_node_overlaps,
        node_overlaps,
        label_overlaps,
        bends,
        edge_length,
        width,
        height,
    }
}

/// Whether segment ab enters the rectangle.
fn segment_hits_rect(a: [f64; 2], b: [f64; 2], r: &Rect) -> bool {
    let inside = |p: [f64; 2]| p[0] > r[0] && p[0] < r[2] && p[1] > r[1] && p[1] < r[3];
    if inside(a) || inside(b) {
        return true;
    }
    let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    (0..4).any(|i| cross(a, b, corners[i], corners[(i + 1) % 4]).is_some())
}

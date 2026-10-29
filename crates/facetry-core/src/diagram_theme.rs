//! A diagram's colours per theme (after Mermaid's `default`, `neutral`, `dark` and `forest`):
//! every colour a style draws with that a node or group does not set itself.

use crate::spec::{DiagramPalette, DiagramTheme};
use crate::{Color, Theme};

pub(crate) fn rgb(hex: u32) -> Color {
    Color::from_srgb8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 0xff)
}

/// The colours of one theme, by style.
#[derive(Debug, Clone)]
pub(crate) struct Look {
    /// A dark page: what is lightened on a light page is darkened on it, and the other way.
    pub dark: bool,
    pub page: Color,
    pub text: Color,
    /// Flat: node, rim and edge.
    pub node: Color,
    pub rim: Color,
    pub edge: Color,
    /// Group bases (flat, glossy, schematic) and outlines (flat, schematic); a schematic
    /// fence's posts and rails.
    pub group: [Color; 3],
    pub group_edge: [Color; 2],
    pub fence: [Color; 2],
    /// Glossy: slab colours by shape (start and end, steps, decisions, stores, preparation,
    /// input and output), arrows, edge-label cards.
    pub shapes: [Color; 6],
    pub arrow: Color,
    pub card: Color,
    /// Glossy: the light pulses moving along animated edges.
    pub pulse: Color,
    /// Isometric: plates, bodies (and the insets labels are printed on), roads, chevrons; floor
    /// cards and their text; node (and group) colours in turn.
    pub plate: Color,
    pub body: Color,
    pub inset: Color,
    pub road: Color,
    pub chevron: Color,
    pub floor_card: Color,
    pub card_text: Color,
    pub caps: [Color; 8],
    /// Schematic: equipment bodies and steel, sign frames and posts, outlines, pipes,
    /// conveyors, supports, flow marks, the floor grid, sign boards and their ink.
    pub equipment: Color,
    pub steel: Color,
    pub frame: Color,
    pub outline: Color,
    pub pipe: Color,
    pub conveyor: Color,
    pub support: Color,
    pub flow: Color,
    pub grid: Color,
    pub sign: Color,
    pub ink: Color,
    /// An end mark that crosses a flow out.
    pub stop: Color,
}

const CAPS: [u32; 8] = [
    0xf6b33f, 0xe8553e, 0x3cb371, 0x3d8fe0, 0x9b5cd6, 0x1fb5a8, 0xf0812f, 0x56677d,
];
const SHAPES: [u32; 6] = [0xe5484d, 0x3e8ef7, 0xd6409f, 0xf5a524, 0x30a46c, 0x8e4ec6];

fn all<const N: usize>(hex: [u32; N]) -> [Color; N] {
    hex.map(rgb)
}

impl Look {
    pub(crate) fn of(theme: DiagramTheme) -> Look {
        let light = Look {
            dark: false,
            page: rgb(0xffffff),
            text: rgb(0x1f2328),
            node: rgb(0xececff),
            rim: rgb(0x9370db),
            edge: rgb(0x4a4a5a),
            group: all([0xf3f5f9, 0xeef1f7, 0xdde1e6]),
            group_edge: all([0x9aa6b8, 0x6f7a89]),
            fence: all([0x9aa4b1, 0xaab3bf]),
            shapes: all(SHAPES),
            arrow: rgb(0x30323a),
            card: rgb(0xf8f8fb),
            pulse: rgb(0x8fd3ff),
            plate: rgb(0xe4e8ee),
            body: rgb(0xf3f5f8),
            inset: rgb(0xffffff),
            road: rgb(0xdadfe6),
            chevron: rgb(0xa3adba),
            floor_card: rgb(0xfbfcfd),
            card_text: rgb(0x6b7280),
            caps: all(CAPS),
            equipment: rgb(0xe9edf2),
            steel: rgb(0xf5f7fa),
            frame: rgb(0x5b6573),
            outline: rgb(0x46505d),
            pipe: rgb(0xb9c2cd),
            conveyor: rgb(0x737d8a),
            support: rgb(0x9aa4b1),
            flow: rgb(0x2fae5a),
            grid: rgb(0xe4e8ed),
            sign: rgb(0xffffff),
            ink: rgb(0x1f2733),
            stop: rgb(0xe8553e),
        };
        match theme {
            DiagramTheme::Default => light,
            DiagramTheme::Neutral => Look {
                node: rgb(0xeeeeee),
                rim: rgb(0x999999),
                edge: rgb(0x666666),
                group: all([0xf4f4f4, 0xf1f1f1, 0xe2e2e2]),
                group_edge: all([0xa3a3a3, 0x7a7a7a]),
                shapes: all([0x52525b, 0x6b7280, 0x3f3f46, 0x9ca3af, 0x71717a, 0x57534e]),
                arrow: rgb(0x3a3a3a),
                pulse: rgb(0xf4f4f5),
                caps: all([
                    0x4b5563, 0x6b7280, 0x9ca3af, 0x374151, 0x52525b, 0x71717a, 0x8b8f97, 0x27272a,
                ]),
                flow: rgb(0x6b7280),
                stop: rgb(0x3f3f46),
                ..light
            },
            DiagramTheme::Forest => Look {
                node: rgb(0xcde498),
                rim: rgb(0x13540c),
                edge: rgb(0x2f6b2f),
                group: all([0xeef7e2, 0xe8f3dc, 0xdbe6d3]),
                group_edge: all([0x6eaa49, 0x5f7356]),
                fence: all([0x8fa585, 0x9fb596]),
                shapes: all([0xc8742c, 0x3c8d40, 0xb5a033, 0x8b5a2b, 0x2e8b57, 0x6b8e23]),
                arrow: rgb(0x23401f),
                pulse: rgb(0xd9f99d),
                card: rgb(0xf6faf0),
                plate: rgb(0xdfe8d6),
                body: rgb(0xf3f7ee),
                road: rgb(0xd5dfcb),
                chevron: rgb(0x9bad8c),
                floor_card: rgb(0xfafcf7),
                card_text: rgb(0x5d6b52),
                caps: all([
                    0x8fbc5a, 0x3c8d40, 0xb5a033, 0x2e8b57, 0x6b8e23, 0x1f9e89, 0xc97b2d, 0x556b2f,
                ]),
                equipment: rgb(0xe6ede0),
                steel: rgb(0xf4f8f0),
                frame: rgb(0x4d5e45),
                outline: rgb(0x3d4a37),
                pipe: rgb(0xb3c2a8),
                conveyor: rgb(0x6c7b62),
                support: rgb(0x95a68a),
                grid: rgb(0xe1e9da),
                ink: rgb(0x1f2a1a),
                ..light
            },
            DiagramTheme::Dark => Look {
                dark: true,
                page: rgb(0x1e2024),
                text: rgb(0xe6e8eb),
                node: rgb(0x2c2f3a),
                rim: rgb(0xa99be8),
                edge: rgb(0xc7cbd3),
                group: all([0x26292f, 0x272a31, 0x2a2e35]),
                group_edge: all([0x5c6573, 0x6b7480]),
                fence: all([0x5a6270, 0x6b7380]),
                arrow: rgb(0xd5d9e0),
                pulse: rgb(0x3e8ef7),
                card: rgb(0x2c2f36),
                plate: rgb(0x2b2e35),
                body: rgb(0x3a3e47),
                inset: rgb(0x2a2d34),
                road: rgb(0x33373e),
                chevron: rgb(0x6f7785),
                floor_card: rgb(0x2a2d34),
                card_text: rgb(0xa3aab6),
                equipment: rgb(0x3b4049),
                steel: rgb(0x4a505a),
                frame: rgb(0x9aa3af),
                outline: rgb(0x14171b),
                pipe: rgb(0x6b7480),
                conveyor: rgb(0xa1aab6),
                support: rgb(0x5a6270),
                flow: rgb(0x3ccf6e),
                grid: rgb(0x2d3137),
                sign: rgb(0x2a2d34),
                ink: rgb(0xe6e8eb),
                ..light
            },
        }
    }

    /// These colours with `palette`'s over them, each role on the colours of every style it
    /// stands for.
    pub(crate) fn with(mut self, palette: &DiagramPalette) -> Look {
        let get = |v: &Option<String>| v.as_deref().and_then(|v| Color::from_hex(v).ok());
        if let Some(c) = get(&palette.page) {
            self.page = c;
        }
        if let Some(c) = get(&palette.text) {
            (self.text, self.ink) = (c, c);
        }
        if let Some(c) = get(&palette.node) {
            (self.node, self.body, self.equipment) = (c, c, c);
        }
        if let Some(c) = get(&palette.node_border) {
            (self.rim, self.outline) = (c, c);
        }
        if let Some(c) = get(&palette.edge) {
            (self.edge, self.arrow, self.chevron, self.pipe) = (c, c, c, c);
            self.road = self.tint(c);
        }
        if let Some(c) = get(&palette.group) {
            self.group = [c; 3];
        }
        if let Some(c) = get(&palette.group_border) {
            self.group_edge = [c; 2];
        }
        if let Some(c) = get(&palette.flow) {
            (self.flow, self.pulse) = (c, c);
        }
        let accents: Vec<Color> = palette
            .accents
            .iter()
            .filter_map(|a| Color::from_hex(a).ok())
            .collect();
        if !accents.is_empty() {
            self.shapes = std::array::from_fn(|i| accents[i % accents.len()]);
            self.caps = std::array::from_fn(|i| accents[i % accents.len()]);
        }
        self
    }

    /// The chart theme a diagram is drawn with: `theme`'s page and text replaced by this
    /// look's, unless the chart was given a page colour of its own (`clearColor`).
    pub(crate) fn chart_theme(&self, theme: &Theme) -> Theme {
        let mut out = *theme;
        if theme.background == Theme::default().background {
            out.background = self.page;
        }
        out.text = self.text;
        out
    }

    /// `c` most of the way to the page: the isometric road under chevrons of colour `c`.
    pub(crate) fn tint(&self, c: Color) -> Color {
        let ([r, g, b, a], [pr, pg, pb, _]) = (c.to_array(), self.page.to_array());
        let m = |v: f32, p: f32| v + (p - v) * 0.6;
        Color::linear(m(r, pr), m(g, pg), m(b, pb), a)
    }

    /// `c` stepped `t` away from the page: darker on a light page, lighter on a dark one.
    pub(crate) fn deepen(&self, c: Color, t: f32) -> Color {
        let [r, g, b, a] = c.to_array();
        let m = |v: f32| {
            if self.dark {
                v + (1.0 - v) * t
            } else {
                v * (1.0 - t)
            }
        };
        Color::linear(m(r), m(g), m(b), a)
    }
}

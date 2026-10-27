# 3D diagrams from Mermaid: design

Status: **D1 implemented** (flowcharts: the add-on, layered layout, the core `diagram` spec and its
rendering); D2 to D5 are still proposals. Where D1 departed from the first draft, this text says
what was built and why (marked *D1:*). This describes how Facetry draws dataflow,
architecture and sequence diagrams, written in a subset of [Mermaid](https://mermaid.js.org)
syntax, as lit 3D scenes with the existing renderer, plus optional animation.

## Goals and non-goals

In priority order:

1. **Layout quality.** The audience is polished, static documentation. A diagram must read at
   a glance: few crossings, few bends, compact, the same picture for the same input, labels that
   never overflow their shapes. It should be measurably better than Mermaid's own layout.
2. **Visual polish.** Diagrams must look good enough to put in product docs without tweaking.
3. **Animation**, opt-in: sequence playback, build-in, flow along edges.

Decided:

| Question | Decision |
|---|---|
| Diagram types | `flowchart` (dataflow), `architecture-beta`, `sequenceDiagram` |
| Mermaid compatibility | A documented subset of each grammar, not full syntax |
| Unsupported or invalid syntax | **Fail** with an error that names the line and column; never render part of a diagram |
| 2D output | Out of scope: diagrams are 3D only |
| Packaging | Separate, opt-in wasm module; the core module stays small |
| Rendering without a GPU (server-side / CI PNGs) | Out of scope: browser only |
| Architecture icons | Mermaid's built-in icons only (`cloud`, `database`, `disk`, `internet`, `server`) |

Out of scope: class, ER, state, gantt, mindmap, timeline and the other Mermaid types; Mermaid
directives, `%%{init}%%` config and `classDef`/`style` beyond what is listed below; icon packs;
editing diagrams interactively.

## Packaging

Two wasm modules cannot share memory or GPU state, so an add-on that rendered on its own would
contain a second copy of wgpu (about 300 KB gzip, 900 KB on WebGL2) for every page that also
shows charts. Instead the add-on does **only CPU work**:

```
 Mermaid text ─▶ facetry_diagrams.wasm ─────────────────▶ ChartSpec ─▶ facetry_wasm (render)
                 parse ─▶ validate ─▶ layout ─▶ route       mark: "diagram", positions resolved
```

* New crate `crates/facetry-diagrams` (parser, layout, routing; no wgpu, no font atlas, only the
  glyph metrics from `facetry-font` for measuring labels) and a wasm-bindgen wrapper built by
  `scripts/build-wasm.sh` as `facetry_diagrams`. Target: under 120 KB gzip.
* The core gains a `diagram` that draws **already laid-out** geometry: positioned nodes
  (with shape, size, label, group), routed edges (polylines or splines with arrowheads),
  platforms for groups, sequence lifelines, activations and fragments. It does no layout, so the
  core cost is limited to the new primitives (below). Target: under 20 KB gzip on the WebGPU
  module, measured with `scripts/wasm-size.sh` like every other milestone.
  *D1:* `diagram` is a top-level field of the spec (next to `data`, `axes`, `layers`) rather than
  a `mark: "diagram"` layer: a diagram has no tables, encodings or axes, and nodes and edges with
  variable-length paths are awkward as columns. A diagram spec has no `data`, `axes`, `layers`
  or `legend`. D1 added 16.5 KB gzip to the WebGPU module (302,667 to 319,196 bytes).
* The new mesh primitive is one generic **prism**: any convex outline extruded upward, lit like
  the bars, with smooth normals for curved outlines. Every node shape is a prism of its outline
  (*D1:* all eleven shapes, including those planned for D2). *D1:* edges are the existing thick
  screen-space 3D polylines (crisp at any zoom), not tubes, and arrowheads are flat prisms, not
  cones.
* npm: a `facetry/diagrams` subpath export that loads the add-on lazily.

```ts
import { Chart, FacetryError } from "facetry";
import { diagram } from "facetry/diagrams"; // fetches facetry_diagrams.wasm on first use
// *D1:* the add-on is 79.8 KB gzip.

try {
  const spec = await diagram.fromMermaid(source, { theme: "default" }); // parse + layout
  const chart = await Chart.create(canvas, spec);
} catch (e) {
  if (e instanceof FacetryError && e.code === "invalid-diagram") {
    for (const { line, column, message } of e.diagramIssues) console.log(line, column, message);
  }
}

await diagram.run(); // renders every <pre class="mermaid"> block, like mermaid.run()
```

Because the output is an ordinary `ChartSpec`, diagrams get spec validation, hover and click,
PNG snapshots, the orbit camera and the label system for free, and a laid-out spec can be cached
or shipped as JSON and rendered without the add-on.

## Errors

`fromMermaid` rejects with a new `FacetryError` code, `invalid-diagram`, and draws nothing. Its
`diagramIssues` lists every problem found (like `issues` for `invalid-spec`), each with a 1-based
`line` and `column`, the source `excerpt`, and a `message`. Syntax that is valid Mermaid but
outside the supported subset is an error of the same kind, with a message that says so
(`"subgraph direction" is not supported`), so users are never shown a silently incomplete
diagram. Unknown icon names, edges to undeclared architecture services and unbalanced sequence
`alt`/`end` blocks are errors too.

## Supported syntax

The exact subset is fixed per diagram type at the start of its milestone, written down in the
README and covered by parser tests; a first cut:

* **`flowchart`** (`graph` too): direction `TB`/`TD`/`BT`/`LR`/`RL`; node shapes `[ ]` rounded
  box, `( )`, `([ ])` stadium, `[( )]` cylinder, `{ }` diamond, `{{ }}` hexagon, `(( ))` circle;
  edges `-->`, `---`, `-.->`, `==>`, with `|label|` and chains `a --> b --> c`, `a & b --> c`;
  `subgraph id [title] … end`, nested; comments `%%`.
* **`architecture-beta`**: `group`, `service` (with `in` for nesting), `junction`, edges with
  sides `a:R --> L:b` and optional arrowheads, the five built-in icons.
* **`sequenceDiagram`**: `participant`/`actor` with `as` aliases, `box … end`, messages `->`,
  `->>`, `-->>`, `-x`, `-)` and their dotted forms, `+`/`-` activations and
  `activate`/`deactivate`, `Note left of|right of|over`, `loop`/`alt`/`else`/`opt`/`par`/`and`/`critical`/`break`
  … `end`, `autonumber`.

## Layout

All layouts are deterministic (no randomness, no dependence on hash order) and sized from the
real glyph metrics, so a node is exactly as large as its padded label.

### Dataflow: layered (Sugiyama)

Done in 2D on the floor plane, then lifted into 3D:

1. Break cycles (depth-first, reverse back edges), merge parallel edges.
2. **Rank** by network simplex (minimises total edge length; dagre's choice).
3. Insert dummy nodes for edges that span ranks.
4. **Order** within ranks: barycenter sweeps plus a transpose pass, keeping the best ordering by
   crossing count.
5. **Coordinates** by Brandes–Köpf (balanced of the four alignments).
6. **Route** edges as splines through the inter-rank gaps, attached at node borders, with
   arrowheads.
7. **Subgraphs** as compound nodes: their members are kept contiguous in each rank and boxed;
   this is the weakest part of Mermaid's output and where most of the effort goes. See
   "Subgraphs (D2)" below for how it is done.

The algorithm follows dagre (MIT), ported and adapted rather than wrapped; ELK's layered
algorithm is better in places but is EPL-licensed and much larger. In 3D, height carries
structure: each subgraph is a platform raised or inset relative to its parent, and where two
edges cross in the plan, one arcs over the other so they never intersect visually.

### Edges arcing over crossings (D2)

Where two edges cross in the plan, the later one (in source order) arcs over the earlier
(`diagram_hops.rs`): crossings are found segment by segment on the routed polylines, leaving
out those within an arc's length (plus what is drawn at the ends: arrowheads, end marks) of the
arcing edge's own ends. Along the arcing edge the height follows a cosine bump of the distance to
the crossing, from 0 at the arc's ends to its full height over the crossing, and the path gets
extra points along the arc so it is smooth. The flat style lifts its lines (a line jump, seen
from above); ribbons (glossy bands, isometric roads, schematic pipes) take a height per point
(`RibbonMark.lift`), tilt their top's normal with the slope and close their underside where they
are raised, so they are bridges. Arc spans and heights are per style: wide enough to clear the
other edge's width, high enough to clear its top (a schematic pipe clears a conveyor).
Chevrons, flow marks and supports follow the bridge.

### Subgraphs (D2)

Done as dagre does clusters, inside the same layered pipeline:

* **Membership.** A node belongs to the innermost subgraph that mentions it (the first to
  close, as in Mermaid). A long edge's dummy nodes belong to the innermost group holding both of
  its ends, so an edge between outsiders is never placed inside a group. An edge to a whole
  subgraph (`a --> sub`) is laid out to the subgraph's first node and cut where it meets the
  box; an empty subgraph gets an invisible stand-in node so it still has a box.
* **Ordering.** After every barycenter sweep each layer is *regrouped*: a group's members are
  gathered into one run, its border nodes at the ends, runs and loose nodes in the order of
  their mean positions (counting only real members: borders would pin the runs in their first
  order). Transposition only swaps two nodes of the same innermost group.
* **Rectangles.** Each group has a border node on each side in every rank it spans, chained down
  the ranks. Brandes–Köpf keeps such chains straight (their segments win type 1 conflicts like
  any long edge's), so the borders are a rectangle's sides; the separation next to a border is
  the group's inner room on that side or the space round it, decided by whether the neighbour is
  inside the group (the right-to-left scans mirror the layers, so sides cannot be read from the
  order).
* **Titles and padding.** Room before a group's first rank and after its last is added between
  the ranks (a group's own plus that of the groups inside it starting on the same rank). The
  title goes on the side that is the top of the drawing: before the first rank (TB), after the
  last (BT), or inside the left border (LR, RL, whose layout x becomes the drawing's y).
* **Drawing.** A base under the nodes, a little thicker per level of nesting and below
  everything that lies on the floor; soft shadows are lifted onto the base they fall on. The
  title is printed at the top left and shrinks if the box is narrow.
* **Measured.** Group overlaps (boxes overlapping without one holding the other, or a box's
  edge cutting a node) and edges through a group holding neither end are in the corpus metrics
  and must stay 0.

### Architecture: constraint grid

Mermaid uses a force-directed layout (Cytoscape fCoSE), which is not reproducible. Architecture
edges already state which side they leave and enter (`api:R --> L:db`), which is a grid
constraint: `db` is right of `api`. Services are placed on an integer grid by propagating these
constraints (breadth-first from the most connected service, resolving conflicts by shifting
columns or rows), groups become the bounding rectangles of their members with margins, and edges
are routed orthogonally between the stated sides. Groups are platforms; nested groups stack.
Services are blocks with their icon on the top face; junctions are small posts.

### Sequence: direct

No search is needed: participants are placed left to right in declaration order, spaced by
their label widths and the widest message between neighbours; each message, note and fragment
takes the next row down. Participant heads are blocks above their lifelines, messages are tubes
with arrowheads (self-messages loop), activations are thin slabs on the lifeline, fragments
(`loop`, `alt`, ...) are translucent frames standing slightly in front of the lifelines with their
label in the corner, `box` groups are tinted platforms behind their participants, notes float in
front. Time runs downwards, as in Mermaid.

### Measuring layout quality

A golden corpus in `examples/diagrams/`: about 50 real-world diagrams per type, taken from public
docs. *D1:* 25 flowcharts written in the style of public documentation (CI pipelines, login
flows, ETL, incident handling, service maps, ...) plus stress cases (cycles, self-loops, fan-out,
dense graphs, every shape and link type); the comparison with Mermaid's own layout is not
automated yet (it needs Mermaid in a browser to measure its text), so the metrics are compared
with the previous layout, diagram by diagram, in the snapshot's diff. For every layout a test computes edge crossings, bends, total edge length, bounding area,
aspect ratio and label overlaps, records them with `insta`, and fails on regressions; a script
prints the same metrics for Mermaid's own layout of the corpus for comparison. Rendered results
go through the existing golden-image test.

## Rendering and polish

* **Camera:** orthographic by default, azimuth about 30, elevation about 35 (close to
  isometric): no perspective distortion, which is what documentation wants. Orbit, zoom and
  reset work as in 3D charts; `camera` in the spec overrides.
  *D1:* azimuth 0 and elevation 50. Turned by any azimuth, the ranks of a layered diagram tilt
  and stop reading as rows, which costs more legibility than the turn adds depth; the front walls
  of the nodes give the 3D look. The starting distance fits the diagram to the canvas, but never
  closer than 1.15 pixels per layout unit, so a small diagram is drawn at about its natural size.
* *D1:* **edges run at the height of the node tops**, not on the floor. On the floor, an edge
  arriving at the far side of a node ends hidden behind the node's own height (in a top-to-bottom
  flowchart, almost every arrowhead); at the top they are visible from any camera above.
* **Contact shadows:** each node and platform gets a soft blurred shadow quad on the
  surface below it. Cheap, and what most of the "solid" look comes from.
* **Shapes:** bevelled, rounded, low-poly instanced meshes, the same lighting as 3D bars.
* **Labels:** node labels on the top face or as camera-facing labels anchored at the node
  (decided by measuring legibility at the default camera); edge labels at the path midpoint,
  lower priority in the existing label-collision pass.
  *D1:* camera-facing labels anchored at the middle of the top face, sized for the starting
  camera (labels keep their pixel size while zooming, so they fit their nodes at the start);
  edge labels at the label rank the layout reserved, with an outline in the background colour.
  Labels that follow the zoom are a D2 polish item.
* **Themes:** presets matching Mermaid's `default`, `neutral`, `dark` and `forest`.
* **Hover and click** report `{ kind: "node" | "edge" | "group" | "message", id }`; hovering a
  dataflow node dims everything outside its upstream and downstream paths.
* **PNG export:** the existing snapshot API, with a `pixelRatio` option for high-DPI images.

## Animation

Opt-in through `animation` in the spec, after layout and polish are done:

* **Sequence playback:** messages, activations and notes appear in order.
* **Build-in:** dataflow nodes rise rank by rank, edges draw from source to target; architecture
  groups first, then services, then edges.
* **Flow along edges:** dashes or particles moving along edge paths.

All three are driven by a single time value in a uniform, so the scene is never rebuilt while
animating. `chart.play()`, `chart.pause()` and `chart.setTime(t)` control it; `setTime` makes
snapshot tests deterministic. Animation honours `prefers-reduced-motion` (shows the end state),
pauses while the canvas is off screen, and when nothing animates the renderer keeps its
render-only-on-change loop. This controller is meant to be the same one keyed transitions on
`chart.update()` will use for charts.

## Milestones

| | Scope |
|---|---|
| **Icons** (done) | Icons of the user's own: `diagram.icons` (stroked and filled SVG path data, viewBox) in the spec, `icons` in `fromMermaid` taking SVG markup, path data or an object; line icons drawn as lines and filled icons filled (holes, both fill rules), in every style |
| **S4** (done) | The schematic style after the cement-plant reference: equipment modelled per shape with drawn edges, pipes and conveyors on supports with flow marks, signs standing on posts with upright printed text, a floor grid |
| **S3** (done) | Floor cards (isometric, opt-in `cards`: title and wrapped text from the label, the node numbered with standing 3D digits, room reserved in the layout, alternating sides) and 32 built-in line icons from `@{ icon: "..." }`, drawn in every style |
| **S2** (done) | The isometric style (pedestals of plate, body, coloured cap and inset; roads with chevrons; palette colours; long shadows; the isometric view), and moving around big diagrams: pan (right-drag, Shift+drag, two fingers), zoom toward the pointer and pinch, closer zoom for diagrams |
| **S1** (done) | The glossy style, chosen by the user ahead of D2 to D5 after comparing with infographic references: `classDef`/`class`/`:::`/`style`, square-cornered routing with ports and tracks, ribbons, solid arrowheads, label cards, gloss, soft shadows, printed (world-space, depth-tested) text, a perspective starting camera. See "Styles" below |
| **D1** (done) | `facetry-diagrams` crate and wasm build, `facetry/diagrams` export, `invalid-diagram` errors; core `diagram` spec with every node shape as a prism, edges and arrowheads; flowchart parser (subset without subgraphs); Sugiyama without subgraphs; quality metrics and the first corpus (25 flowcharts) |
| **D2** (in progress) | Done: subgraphs (parsing with nesting, `direction` and edges to a subgraph; compound layout; group bases in every style; group metrics), schematic signs that turn to the camera, edges arcing over crossings. Next: `diagram.run()`; later, labels that follow the zoom and themes |
| **D3** | Sequence: parser, layout, activations, fragments, notes, boxes |
| **D4** | Architecture: parser, constraint-grid layout, orthogonal routing, icon atlas, groups, junctions |
| **D5** | Animation controller, sequence playback, build-in, flow along edges |

Each milestone reports the size of both core modules and of the add-on, as `docs/wasm-size.md`
does for charts.

Order agreed after D1: **styles first** (S1 glossy, then an isometric-infographic style with
pedestals, floor cards and icons, then a schematic one), then subgraphs, sequence and
architecture.

## Styles

Looking at reference infographics showed that what makes 3D diagrams appealing is mostly style
(materials, shadows, thick geometry, text on surfaces), not layout. A style is a whole look:
`diagram.style` in the spec selects how the core draws, and the add-on's `style` option also
selects routing, because some looks need square-cornered edges.

**Glossy (S1).** What it changed in the D1 decisions:

* **Camera:** perspective, azimuth 28 and elevation 40. D1 rejected a turned camera because
  floating, screen-facing labels and tilted rows read badly; glossy labels are *printed* on the
  slabs (laid out in world units on the horizontal plane, depth-tested), so they turn with the
  scene and stay readable, and the turn is the look. The starting distance is fitted by
  projecting the plot box's corners through the actual lens.
* **Edges** lie on the floor as solid ribbons with solid arrowheads, stopping short of the nodes
  (on the floor, the last stretch behind a slab is hidden by it; the gap keeps the arrowhead in
  view from the starting elevation). They are routed at right angles: ports spread along a
  node's bottom and top sides in the order of where the edges go, one turn per change of column
  in the gap between ranks, tracks per gap ordered so turns do not cross needlessly, a lead-in
  long enough for the arrowhead, corners rounded. Dummy nodes only constrain the route where they
  carry a label or sit in a rank with real nodes.
* **Materials:** a per-vertex gloss in the lit-mesh shader gives each top face a sheen gradient
  (brighter at the far side) plus a camera-following specular term, and a lighter catch-light
  rim along each slab's top edge. Soft shadows are translucent rings under slabs and cards.
* **Colour:** by shape (start/end, step, decision, store, ...) unless `classDef`/`style` set
  `fill`; `stroke`, `stroke-width` and `color` are honoured in both styles.

Known limits: square-cornered routing crosses more than curves (15 against 9 in the corpus);
edges into a narrow node from many sources crowd its top side; wide diagrams print small labels
at the starting distance (they grow when zooming, being printed).

**Isometric (S2).** After the infographic-timeline reference: each node is a stack of four
solids of its own shape (plate, body, cap, inset), so circles become the round pedestals of the
reference and other shapes become tiles of the same make; the cap is the node's mark (hover,
picking), the rest decoration. Edges reuse the glossy routing but are drawn as roads lying on the
floor that run under the pedestals: a pedestal hides the end of a road, so direction is shown by
chevrons painted along the road and a darker one just before the head, rather than by an
arrowhead at the node. Labels are printed on the inset, so in the 45-degree view they run along
the drawing's axes like the text of a printed isometric chart. Cards, icons and standing
numerals came in S3 (below).

**Cards and icons (S3).** Cards are opt-in (`cards: true`) because they change what a node shows:
the first line of the label becomes the card's title, the rest its text (wrapped to 170 units),
and the pedestal shows the node's number. The layout treats a node and its card as
one footprint, wide along the rank, so ranks space themselves for the cards and the Sugiyama
ordering is unchanged; afterwards the node and the card are pulled to opposite ends of the
footprint, the card's side alternating every rank (the reference's zigzag), and edges are routed
to the node's own centre and size. The metrics count cards as obstacles: in the corpus no card
overlaps a node or another card and no edge crosses one. A card is a thin white prism with a soft
shadow, its title and text printed on it, and the icon at its outer edge.

The number stands on the pedestal as solid digits in the node's colour, facing the isometric
view (it is a solid, so orbiting shows its sides and back rather than turning it). Prisms could
only rise from the floor, so a prism can now be *stood*: its outline is drawn in a vertical plane
and extruded across it, closed at both ends. A digit is a monoline face in the icons' path
language, and each stroke segment becomes a bar and each joint a disc of that thickness; the
pieces overlap, so their walls stop just short of the faces (or the depth test would draw their
seams) and their faces share one sheen. Labels that are not a number of up to three digits are
printed instead.

Icons are line drawings in a small path language (move, line, close, circles, ellipses, arcs,
rounded boxes) in a 24-unit box, drawn as polylines on the surface they belong to: above the
label on a node, on the card with cards. 32 icons cover what infographics and flowcharts mostly
show (people, data, servers, money, time, alerts, ...); `fa:` prefixes are dropped and a few common
names alias onto them, and an unknown name is an error rather than a missing picture. This is not
the architecture icon atlas of D4, which still needs Mermaid's own icon names.

**Schematic (S4).** After the cement-plant reference, whose appeal is that it reads as a
technical illustration: recognisable equipment, pipework, signs, and almost no colour. In the
same isometric view as S2:

* **Equipment by shape.** A node is a small model built from prisms: a building (rectangle),
  a silo on a square plinth with a stepped roof (cylinder), a banded tank (circle), a valve with
  its hand wheel (diamond), a reactor with a chimney (hexagon), a drum with bands (stadium).
  Its first solid, the body, has the node's footprint and is its mark; every body is at least as
  tall as a pipe, so pipes always end in a wall. Pale greys throughout (a `fill` recolours the
  equipment).
* **Drawn edges.** Lighting alone left light grey walls hard to tell apart, so every solid gets
  dark edges as one polyline: a ring round its top, one round its foot, and its corners for
  solids that have them; the depth test hides the ones behind.
* **Pipes** are ribbons raised off the floor on square supports every 48 units (none inside
  equipment), with green flow marks on top and a darker one before the head; thick edges are
  wider, dark conveyors, dotted edges dashed pipes.
* **Signs.** Labels stand on white boards in a dark frame, on posts above the equipment, facing
  the starting view; edge labels get smaller ones on the pipe. Text printed upright needed one
  more thing in the renderer: a printed glyph can now lie in a vertical plane (`Text3.upright`,
  the plane's reading direction) as well as on the floor. Icons are drawn on the sign in the same
  plane. Boards and frames are standing prisms (from S3). A sign turns about its post to face
  the camera as it orbits (`Scene3d.signs`: built facing the isometric view, turned about the
  pivot by the renderer whenever the azimuth changes, then re-tessellated with its text and
  icon), and its face is printed unlit, so it reads white from every side.
* **Grid.** A faint square grid over the floor, like drafting paper, and soft contact shadows.

Known limits: signs are as wide as their text and float at the equipment's height, so signs of
nodes close together in the view can overlap in the starting view; the routing is the glossy
style's, so it shares its crossing count.

**Icons of the user's own.** The 32 built-in icons cannot cover every domain (pumps, valves,
a company's own symbols), so icons can be brought as SVG, the format every icon set ships in:

* **In the spec**, `diagram.icons` maps a name to `{ d, viewBox? }`: SVG path data and the area
  it is drawn in. The spec stays self-contained (a stored diagram draws without the add-on or
  the icon files), and only path data has to be understood by the wasm. A custom icon wins over
  a built-in one of the same name; nodes may only name icons that exist, and a broken path is
  reported at `/diagram/icons/<name>/d` with where it goes wrong.
* **Path data** is read by a small reader in the core (`svg_path.rs`): every command, absolute
  and relative, the packed number forms (`1.5.5`, `1-2`, arc flags run together), cubic and
  quadratic curves (smooth ones reflected) cut into 10 steps and arcs per SVG's endpoint
  parameterisation in steps of at most 15 degrees. It reads numbers itself, since
  `str::parse::<f64>` would link the standard float parser (14 KB) into the add-on. A path may
  come to 4,000 points at most. The strokes are fitted into the built-in icons' 24-unit box,
  centred, so every style draws them the same way: on nodes, cards and signs.
* **SVG markup** is turned into path data in TypeScript with the browser's own parser
  (`iconFromSvg`), so no XML parser enters the wasm: shapes become path data (circles and
  ellipses as two arcs, rounded rects with corner arcs, polylines), unpainted shapes such as an
  icon set's bounding box are dropped, and what cannot be placed exactly without more code
  (`transform`, `<use>`, `<text>`, `<image>`) is refused with a remedy.
* **Lines and fills.** An icon has `stroke` path data, drawn as lines like the built-in
  icons, and `fill`, a list of shapes each filled on its own (SVG fills each element apart, and
  joining them into one path would let opposite windings cut holes), with SVG's fill rules.
  The markup reader sorts every shape by how SVG would paint it; bare path data counts as a
  fill, since the sets that publish bare paths are filled ones.
* **Filling** (`fill.rs`): the plane is cut into bands at every vertex's height; within a band
  the crossing edges are ordered and the fill rule picks the inside spans, each a trapezoid.
  No ear clipping and no special cases for holes or touching contours. The fills are flat
  triangles printed on the surface (`Scene3d.fills`), drawn by the lit-mesh pipeline with a zero
  normal, which the shader takes as unlit ink. The diagrams add-on only checks that an icon's
  paths read; the triangulation lives in the core.

**Moving around (S2).** Diagrams are maps more than objects, so orbit alone is not enough: the
camera got a pan (the point looked at moves across the floor, clamped to the plot box) and zoom
toward the pointer (the point of the floor under it is kept there by panning after each zoom
step), and diagrams may come as close as 0.15 box radii. Touch gets pinch (zoom about the
fingers' midpoint) and two-finger pan. Charts gain the same pan and pointer zoom but keep their
closest distance.

//! Ranking: which layer each node goes in. Cycles are broken first (some edges are treated
//! as reversed), then ranks are assigned by **network simplex** (Gansner et al., "A Technique
//! for Drawing Directed Graphs", 1993; dagre and Graphviz `dot` use it too): among all ranks
//! where every edge `u -> v` has `rank(v) - rank(u) >= minlen`, it finds one with the least
//! total weighted edge length, so edges are as short as they can be.

/// An edge for ranking: `from -> to` must span at least `minlen` ranks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RankEdge {
    pub from: usize,
    pub to: usize,
    pub minlen: i32,
    pub weight: f64,
}

/// Which edges to reverse so the graph has no cycle: a depth-first search from every node in
/// input order, reversing each edge that leads back onto the current path. Self-loops are
/// never reversed (the caller leaves them out of ranking). Deterministic, and it keeps the
/// author's order: the first-declared path through a cycle runs forward.
pub fn feedback_edges(nodes: usize, edges: &[(usize, usize)]) -> Vec<bool> {
    let mut out_edges = vec![Vec::new(); nodes];
    for (i, &(u, v)) in edges.iter().enumerate() {
        if u != v {
            out_edges[u].push(i);
        }
    }
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        New,
        OnPath,
        Done,
    }
    let mut state = vec![State::New; nodes];
    let mut reversed = vec![false; edges.len()];
    for start in 0..nodes {
        if state[start] != State::New {
            continue;
        }
        // Iterative DFS: (node, next out-edge index).
        let mut stack = vec![(start, 0usize)];
        state[start] = State::OnPath;
        while let Some(&mut (u, ref mut k)) = stack.last_mut() {
            if let Some(&e) = out_edges[u].get(*k) {
                *k += 1;
                let v = edges[e].1;
                match state[v] {
                    State::New => {
                        state[v] = State::OnPath;
                        stack.push((v, 0));
                    }
                    State::OnPath => reversed[e] = true,
                    State::Done => {}
                }
            } else {
                state[u] = State::Done;
                stack.pop();
            }
        }
    }
    reversed
}

/// Optimal ranks for an acyclic graph, the smallest 0 in every connected component.
pub fn network_simplex(nodes: usize, edges: &[RankEdge]) -> Vec<i32> {
    let mut rank = vec![0; nodes];
    for component in components(nodes, edges) {
        let index: Vec<Option<usize>> = {
            let mut index = vec![None; nodes];
            for (i, &v) in component.iter().enumerate() {
                index[v] = Some(i);
            }
            index
        };
        let local: Vec<RankEdge> = edges
            .iter()
            .filter_map(|e| {
                Some(RankEdge {
                    from: index[e.from]?,
                    to: index[e.to]?,
                    ..*e
                })
            })
            .collect();
        let ranks = Simplex::new(component.len(), &local).solve();
        for (i, &v) in component.iter().enumerate() {
            rank[v] = ranks[i];
        }
    }
    rank
}

/// The weakly connected components, each in node order, ordered by their first node.
fn components(nodes: usize, edges: &[RankEdge]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..nodes).collect();
    fn find(parent: &mut [usize], mut v: usize) -> usize {
        while parent[v] != v {
            parent[v] = parent[parent[v]];
            v = parent[v];
        }
        v
    }
    for e in edges {
        let (a, b) = (find(&mut parent, e.from), find(&mut parent, e.to));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut slot = vec![usize::MAX; nodes];
    for v in 0..nodes {
        let r = find(&mut parent, v);
        if slot[r] == usize::MAX {
            slot[r] = groups.len();
            groups.push(Vec::new());
        }
        groups[slot[r]].push(v);
    }
    groups
}

/// Network simplex on one connected, acyclic graph.
struct Simplex<'a> {
    n: usize,
    edges: &'a [RankEdge],
    rank: Vec<i32>,
    in_tree: Vec<bool>,
    /// Edges incident to each node.
    incident: Vec<Vec<usize>>,
}

impl<'a> Simplex<'a> {
    fn new(n: usize, edges: &'a [RankEdge]) -> Self {
        let mut incident = vec![Vec::new(); n];
        for (i, e) in edges.iter().enumerate() {
            incident[e.from].push(i);
            incident[e.to].push(i);
        }
        Self {
            n,
            edges,
            rank: vec![0; n],
            in_tree: vec![false; edges.len()],
            incident,
        }
    }

    fn slack(&self, e: usize) -> i32 {
        let e = &self.edges[e];
        self.rank[e.to] - self.rank[e.from] - e.minlen
    }

    fn solve(mut self) -> Vec<i32> {
        if self.n <= 1 {
            return vec![0; self.n];
        }
        self.longest_path();
        self.feasible_tree();
        // Each exchange strictly lowers the cost except in degenerate cases; the cap keeps a
        // degenerate cycle from looping for ever.
        let limit = 20 * self.n + 100;
        for _ in 0..limit {
            let cut = self.cut_values();
            let Some(leave) = (0..self.edges.len())
                .filter(|&e| self.in_tree[e] && cut[e] < -1e-9)
                .min_by(|&a, &b| cut[a].total_cmp(&cut[b]).then(a.cmp(&b)))
            else {
                break;
            };
            let Some(enter) = self.entering(leave) else {
                break;
            };
            self.in_tree[leave] = false;
            self.in_tree[enter] = true;
            self.ranks_from_tree();
        }
        let min = self.rank.iter().copied().min().unwrap_or(0);
        self.rank.iter().map(|r| r - min).collect()
    }

    /// Initial ranks: sinks at 0, every other node as low as its successors allow.
    fn longest_path(&mut self) {
        let mut done = vec![false; self.n];
        let mut out = vec![Vec::new(); self.n];
        for (i, e) in self.edges.iter().enumerate() {
            out[e.from].push(i);
        }
        for start in 0..self.n {
            if done[start] {
                continue;
            }
            let mut stack = vec![(start, false)];
            while let Some((v, expanded)) = stack.pop() {
                if done[v] {
                    continue;
                }
                if expanded {
                    let r = out[v]
                        .iter()
                        .map(|&e| self.rank[self.edges[e].to] - self.edges[e].minlen)
                        .min()
                        .unwrap_or(0);
                    self.rank[v] = r;
                    done[v] = true;
                } else {
                    stack.push((v, true));
                    for &e in &out[v] {
                        let w = self.edges[e].to;
                        if !done[w] {
                            stack.push((w, false));
                        }
                    }
                }
            }
        }
    }

    /// A spanning tree of tight edges (slack 0), shifting ranks until one exists.
    fn feasible_tree(&mut self) {
        let mut in_node = vec![false; self.n];
        in_node[0] = true;
        let mut size = 1;
        loop {
            // Grow along tight edges.
            let mut stack: Vec<usize> = (0..self.n).filter(|&v| in_node[v]).collect();
            while let Some(v) = stack.pop() {
                for &e in &self.incident[v] {
                    let w = if self.edges[e].from == v {
                        self.edges[e].to
                    } else {
                        self.edges[e].from
                    };
                    if !in_node[w] && self.slack(e) == 0 {
                        in_node[w] = true;
                        self.in_tree[e] = true;
                        size += 1;
                        stack.push(w);
                    }
                }
            }
            if size >= self.n {
                return;
            }
            // The edge with one end in the tree and the least slack; move the tree to make it
            // tight.
            let Some(e) = (0..self.edges.len())
                .filter(|&e| in_node[self.edges[e].from] != in_node[self.edges[e].to])
                .min_by_key(|&e| (self.slack(e), e))
            else {
                return; // not connected: cannot happen for one component
            };
            let delta = if in_node[self.edges[e].from] {
                self.slack(e)
            } else {
                -self.slack(e)
            };
            for (rank, _) in self.rank.iter_mut().zip(&in_node).filter(|(_, t)| **t) {
                *rank += delta;
            }
        }
    }

    /// Tree adjacency: for each node, its tree edges.
    fn tree_adjacency(&self) -> Vec<Vec<usize>> {
        let mut adj = vec![Vec::new(); self.n];
        for (e, edge) in self.edges.iter().enumerate() {
            if self.in_tree[e] {
                adj[edge.from].push(e);
                adj[edge.to].push(e);
            }
        }
        adj
    }

    /// Post-order numbering of the tree rooted at node 0: `lim[v]` is v's number and
    /// `low[v]` the smallest number in v's subtree, so `w` is under `v` iff
    /// `low[v] <= lim[w] <= lim[v]`. Also each node's parent edge.
    fn numbering(&self) -> (Vec<usize>, Vec<usize>, Vec<Option<usize>>) {
        let adj = self.tree_adjacency();
        let (mut low, mut lim) = (vec![0; self.n], vec![0; self.n]);
        let mut parent = vec![None; self.n];
        let mut next = 0;
        let mut seen = vec![false; self.n];
        let mut stack = vec![(0usize, 0usize)];
        seen[0] = true;
        let mut low_stack = vec![0usize];
        while let Some(&mut (v, ref mut k)) = stack.last_mut() {
            if let Some(&e) = adj[v].get(*k) {
                *k += 1;
                let w = if self.edges[e].from == v {
                    self.edges[e].to
                } else {
                    self.edges[e].from
                };
                if !seen[w] {
                    seen[w] = true;
                    parent[w] = Some(e);
                    stack.push((w, 0));
                    low_stack.push(next);
                }
            } else {
                stack.pop();
                low[v] = low_stack.pop().unwrap_or(next);
                lim[v] = next;
                next += 1;
            }
        }
        (low, lim, parent)
    }

    /// The cut value of every tree edge (others are 0): the weight of the edges crossing from
    /// the edge's tail component to its head component, minus the weight crossing back.
    fn cut_values(&self) -> Vec<f64> {
        let (low, lim, parent) = self.numbering();
        let mut cut = vec![0.0; self.edges.len()];
        for v in 0..self.n {
            let Some(te) = parent[v] else { continue };
            // Removing `te` splits off v's subtree.
            let under = |w: usize| low[v] <= lim[w] && lim[w] <= lim[v];
            // Is the subtree the tree edge's tail (from) side or its head side?
            let subtree_is_tail = self.edges[te].from == v;
            let mut value = 0.0;
            for e in self.edges {
                let (a, b) = (under(e.from), under(e.to));
                if a == b {
                    continue;
                }
                // Crossing tail -> head counts +, head -> tail counts -.
                let from_tail = if subtree_is_tail { a } else { b };
                value += if from_tail { e.weight } else { -e.weight };
            }
            cut[te] = value;
        }
        cut
    }

    /// The non-tree edge to replace `leave` with: of those crossing from the head component
    /// to the tail component, the one with least slack.
    fn entering(&self, leave: usize) -> Option<usize> {
        let (low, lim, parent) = self.numbering();
        let e = &self.edges[leave];
        // The child end of the leaving edge roots the split-off subtree.
        let child = if parent[e.to] == Some(leave) {
            e.to
        } else {
            e.from
        };
        let under = |w: usize| low[child] <= lim[w] && lim[w] <= lim[child];
        let subtree_is_tail = child == e.from;
        (0..self.edges.len())
            .filter(|&f| !self.in_tree[f])
            .filter(|&f| {
                let (a, b) = (under(self.edges[f].from), under(self.edges[f].to));
                if a == b {
                    return false;
                }
                // Must go head component -> tail component.
                let from_tail = if subtree_is_tail { a } else { b };
                !from_tail
            })
            .min_by_key(|&f| (self.slack(f), f))
    }

    /// Ranks that make every tree edge tight, from node 0 outwards.
    fn ranks_from_tree(&mut self) {
        let adj = self.tree_adjacency();
        let mut seen = vec![false; self.n];
        seen[0] = true;
        let mut stack = vec![0];
        while let Some(v) = stack.pop() {
            for &e in &adj[v] {
                let edge = self.edges[e];
                let (w, r) = if edge.from == v {
                    (edge.to, self.rank[v] + edge.minlen)
                } else {
                    (edge.from, self.rank[v] - edge.minlen)
                };
                if !seen[w] {
                    seen[w] = true;
                    self.rank[w] = r;
                    stack.push(w);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(from: usize, to: usize) -> RankEdge {
        RankEdge {
            from,
            to,
            minlen: 1,
            weight: 1.0,
        }
    }

    fn cost(rank: &[i32], edges: &[RankEdge]) -> f64 {
        edges
            .iter()
            .map(|e| f64::from(rank[e.to] - rank[e.from]) * e.weight)
            .sum()
    }

    fn feasible(rank: &[i32], edges: &[RankEdge]) -> bool {
        edges.iter().all(|e| rank[e.to] - rank[e.from] >= e.minlen)
    }

    #[test]
    fn a_chain_gets_consecutive_ranks() {
        let edges = [e(0, 1), e(1, 2), e(2, 3)];
        assert_eq!(network_simplex(4, &edges), [0, 1, 2, 3]);
    }

    #[test]
    fn a_short_branch_hangs_next_to_its_neighbour_not_at_the_top() {
        // 0 -> 1 -> 2 -> 3 and 4 -> 3: longest path would put 4 at rank 0, but rank 2 makes
        // its edge length 1.
        let edges = [e(0, 1), e(1, 2), e(2, 3), e(4, 3)];
        let r = network_simplex(5, &edges);
        assert_eq!(r, [0, 1, 2, 3, 2]);
    }

    #[test]
    fn the_gansner_example_reaches_the_optimum() {
        // The graph of figure 2-2 in the paper (a..h = 0..7), optimal cost 10.
        let edges = [
            e(0, 1),
            e(1, 2),
            e(2, 3),
            e(3, 7),
            e(0, 4),
            e(0, 5),
            e(4, 6),
            e(5, 6),
            e(6, 7),
        ];
        let r = network_simplex(8, &edges);
        assert!(feasible(&r, &edges));
        assert_eq!(cost(&r, &edges), 10.0);
    }

    #[test]
    fn minlen_and_weights_are_respected() {
        let edges = [
            RankEdge {
                minlen: 2,
                ..e(0, 1)
            },
            e(1, 2),
            RankEdge {
                weight: 5.0,
                ..e(0, 2)
            },
        ];
        let r = network_simplex(3, &edges);
        assert!(feasible(&r, &edges));
        assert_eq!(r, [0, 2, 3]);
    }

    #[test]
    fn components_start_at_zero_each() {
        let edges = [e(0, 1), e(2, 3), e(3, 4)];
        assert_eq!(network_simplex(6, &edges), [0, 1, 0, 1, 2, 0]);
    }

    #[test]
    fn random_graphs_are_feasible_and_no_worse_than_longest_path() {
        // A small deterministic generator (no rand dependency).
        let mut seed = 12345u64;
        let mut next = |m: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % m
        };
        for _ in 0..200 {
            let n = 2 + next(14) as usize;
            let mut edges = Vec::new();
            for _ in 0..next(3 * n as u64) {
                let (a, b) = (next(n as u64) as usize, next(n as u64) as usize);
                if a < b {
                    edges.push(RankEdge {
                        minlen: 1 + next(2) as i32,
                        weight: 1.0 + next(3) as f64,
                        ..e(a, b)
                    });
                }
            }
            let r = network_simplex(n, &edges);
            assert!(feasible(&r, &edges), "{edges:?} {r:?}");
            let mut lp = Simplex::new(n, &edges);
            lp.longest_path();
            assert!(cost(&r, &edges) <= cost(&lp.rank, &edges) + 1e-9);
        }
    }

    #[test]
    fn cycles_are_broken_against_input_order() {
        // 0 -> 1 -> 2 -> 0: the edge back to 0 is reversed.
        let rev = feedback_edges(3, &[(0, 1), (1, 2), (2, 0)]);
        assert_eq!(rev, [false, false, true]);
        // Two-cycles and self-loops.
        let rev = feedback_edges(2, &[(0, 1), (1, 0), (1, 1)]);
        assert_eq!(rev, [false, true, false]);
    }
}

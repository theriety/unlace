//! The minimum feedback arc set of one import-graph component, deterministic
//! by construction: integer weights, no time budget, and every tie broken on
//! the files' path order.
//!
//! A component of up to [`EXACT_LIMIT`] files is solved exactly by dynamic
//! programming over file subsets: a cut is the set of backward edges of some
//! linear order, and the cheapest order is built one file at a time. Ties keep
//! the cut with fewer imports, then the one whose `(importer, imported)` list
//! sorts first. Larger components take a greedy Eades–Lin–Smyth order whose
//! choices also fall to the lowest index, then restore every cut edge that no
//! longer closes a cycle. The order costs O((V+E)·log V) and the restore pass
//! O(c·(V+E)) for a cut of c imports, both on adjacency lists.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::iter;

/// The largest component solved exactly: 2^12 subsets.
pub(super) const EXACT_LIMIT: usize = 12;

/// The imports to remove, as local `(importer, imported)` indices ascending,
/// and whether the set is proven minimum.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Cut {
    /// The edges to remove, ascending.
    pub(super) edges: Vec<(usize, usize)>,
    /// `true` for the exact solve.
    pub(super) exact: bool,
}

/// Solves the component of `count` files whose edges carry `weights`.
pub(super) fn solve(count: usize, weights: &BTreeMap<(usize, usize), u64>) -> Cut {
    if count <= EXACT_LIMIT {
        Cut {
            edges: exact(count, weights),
            exact: true,
        }
    } else {
        Cut {
            edges: greedy(count, weights),
            exact: false,
        }
    }
}

/// One subset's cheapest prefix order, by its backward edges.
#[derive(Clone, Default)]
struct Prefix {
    cost: u64,
    cut: Vec<(usize, usize)>,
}

impl Prefix {
    /// The ordering key: cost, then import count, then the sorted edge list.
    /// Cost and count are additive, so the exact cut is optimal on both; the
    /// list only makes the choice deterministic, compared per subset as the
    /// cut grows, and is not a global lexicographic minimum.
    fn key(&self) -> (u64, usize, &[(usize, usize)]) {
        (self.cost, self.cut.len(), self.cut.as_slice())
    }
}

/// The exact cut: the cheapest backward-edge set over every linear order.
///
/// Subsets are taken smallest first. A subset only grows into larger ones, so
/// when one is taken every subset it grows from has offered its candidate and
/// its prefix is final; the last subset taken is the full set.
pub(super) fn exact(count: usize, weights: &BTreeMap<(usize, usize), u64>) -> Vec<(usize, usize)> {
    let mut best: BTreeMap<usize, Prefix> = BTreeMap::new();
    best.insert(0, Prefix::default());
    let mut last = Prefix::default();
    while let Some((placed, prefix)) = best.pop_first() {
        for next in (0..count).filter(|next| placed & (1 << next) == 0) {
            // `next` sits after every placed file, so its edges into them run
            // backward and must be cut.
            let mut candidate = prefix.clone();
            for earlier in (0..count).filter(|earlier| placed & (1 << earlier) != 0) {
                if let Some(weight) = weights.get(&(next, earlier)) {
                    candidate.cost += weight;
                    candidate.cut.push((next, earlier));
                }
            }
            candidate.cut.sort_unstable();
            let grown = placed | (1 << next);
            if best
                .get(&grown)
                .is_none_or(|current| candidate.key() < current.key())
            {
                best.insert(grown, candidate);
            }
        }
        last = prefix;
    }
    last.cut
}

/// One edge as one of its files sees it.
#[derive(Clone, Copy)]
struct Arc {
    /// The edge's index in [`Adjacency::edges`].
    edge: usize,
    /// The file at the edge's other end.
    file: usize,
    /// The edge's weight.
    weight: u64,
}

/// The component's edges as adjacency lists.
///
/// Edges are numbered in ascending `(importer, imported)` order, so each
/// list is ascending by neighbour and every walk visits files in path order.
/// Self-edges are left out: they never order two files.
struct Adjacency {
    /// Each edge as `(importer, imported, weight)`, ascending.
    edges: Vec<(usize, usize, u64)>,
    /// The edges leaving each file, each naming its imported file.
    outgoing: Vec<Vec<Arc>>,
    /// The edges entering each file, each naming its importer.
    incoming: Vec<Vec<Arc>>,
}

impl Adjacency {
    fn new(count: usize, weights: &BTreeMap<(usize, usize), u64>) -> Self {
        let edges: Vec<(usize, usize, u64)> = weights
            .iter()
            .filter(|((importer, imported), _)| {
                importer != imported && *importer < count && *imported < count
            })
            .map(|(&(importer, imported), &weight)| (importer, imported, weight))
            .collect();
        // the edges ascend by importer, so each file's outgoing edges are one run.
        let mut by_importer = edges.iter().enumerate().peekable();
        let outgoing = (0..count)
            .map(|file| {
                iter::from_fn(|| by_importer.next_if(|&(_, &(importer, _, _))| importer == file))
                    .map(|(edge, &(_, imported, weight))| Arc {
                        edge,
                        file: imported,
                        weight,
                    })
                    .collect()
            })
            .collect();
        // sorted by imported file, then edge, each file's incoming edges are one
        // run, ascending by edge.
        let mut entering: Vec<(usize, Arc)> = edges
            .iter()
            .enumerate()
            .map(|(edge, &(importer, imported, weight))| {
                let arc = Arc {
                    edge,
                    file: importer,
                    weight,
                };
                (imported, arc)
            })
            .collect();
        entering.sort_unstable_by_key(|&(imported, arc)| (imported, arc.edge));
        let mut by_imported = entering.into_iter().peekable();
        let incoming = (0..count)
            .map(|file| {
                iter::from_fn(|| by_imported.next_if(|&(imported, _)| imported == file))
                    .map(|(_, arc)| arc)
                    .collect()
            })
            .collect();
        Self {
            edges,
            outgoing,
            incoming,
        }
    }
}

/// The heuristic cut: the backward edges of a greedy order, then every cut
/// edge restored that closes no cycle, heaviest first.
///
/// The kept edges stay acyclic throughout: they start as the forward edges of
/// a linear order, and an edge is restored only when it closes no cycle. So
/// restoring `importer -> imported` is safe exactly when `imported` does not
/// already reach `importer`, which one depth-first walk answers. The pass
/// costs O(c·(V+E)) for a cut of c edges.
pub(super) fn greedy(count: usize, weights: &BTreeMap<(usize, usize), u64>) -> Vec<(usize, usize)> {
    let graph = Adjacency::new(count, weights);
    // the order places each of the `count` files once, so its ranks sorted by
    // file hold each file's rank at the file's own index.
    let mut ranked: Vec<(usize, usize)> = greedy_order(&graph)
        .into_iter()
        .enumerate()
        .map(|(rank, file)| (file, rank))
        .collect();
    ranked.sort_unstable();
    let position: Vec<usize> = ranked.into_iter().map(|(_, rank)| rank).collect();
    let rank = |file: usize| position.get(file).copied().unwrap_or_default();
    let mut in_cut: Vec<bool> = graph
        .edges
        .iter()
        .map(|&(importer, imported, _)| rank(importer) > rank(imported))
        .collect();
    let mut restore: Vec<(usize, (usize, usize, u64))> = graph
        .edges
        .iter()
        .zip(&in_cut)
        .enumerate()
        .filter(|&(_, (_, &cut))| cut)
        .map(|(edge, (&endpoints, _))| (edge, endpoints))
        .collect();
    // heaviest first, then the edge order, which is (importer, imported).
    restore.sort_by_key(|&(edge, (_, _, weight))| (Reverse(weight), edge));
    let mut walk = Walk::new(count);
    for (edge, (importer, imported, _)) in restore {
        if !walk.reaches(&graph, &in_cut, imported, importer)
            && let Some(flag) = in_cut.get_mut(edge)
        {
            *flag = false;
        }
    }
    graph
        .edges
        .iter()
        .zip(&in_cut)
        .filter(|(_, cut)| **cut)
        .map(|(&(importer, imported, _), _)| (importer, imported))
        .collect()
}

/// A reusable depth-first walk over the kept edges. Each walk stamps the
/// files it visits, so no walk clears the previous one's marks.
struct Walk {
    /// The walk that last visited each file.
    visited: Vec<u32>,
    /// The current walk's stamp.
    stamp: u32,
    /// The files still to expand.
    stack: Vec<usize>,
}

impl Walk {
    fn new(count: usize) -> Self {
        Self {
            visited: vec![0; count],
            stamp: 0,
            stack: Vec::new(),
        }
    }

    /// Whether `from` reaches `to` over the edges not in the cut.
    fn reaches(&mut self, graph: &Adjacency, in_cut: &[bool], from: usize, to: usize) -> bool {
        self.stamp = self.stamp.wrapping_add(1);
        if self.stamp == 0 {
            self.visited.fill(0);
            self.stamp = 1;
        }
        self.stack.clear();
        self.enter(from);
        while let Some(file) = self.stack.pop() {
            if file == to {
                return true;
            }
            for arc in graph.outgoing.get(file).into_iter().flatten() {
                if !in_cut.get(arc.edge).copied().unwrap_or(true) {
                    self.enter(arc.file);
                }
            }
        }
        false
    }

    /// Stamps `file` and queues it to expand, unless this walk already did.
    fn enter(&mut self, file: usize) {
        if let Some(mark) = self.visited.get_mut(file)
            && *mark != self.stamp
        {
            *mark = self.stamp;
            self.stack.push(file);
        }
    }
}

/// What a file not yet placed still sends to and receives from the other
/// files not yet placed.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Weights {
    sent: u64,
    received: u64,
}

impl Weights {
    /// The outgoing-minus-incoming weight.
    fn delta(self) -> i128 {
        i128::from(self.sent) - i128::from(self.received)
    }
}

/// The files not yet placed, bucketed for an Eades–Lin–Smyth order.
///
/// Every remaining file sits in one bucket keyed by its outgoing-minus-incoming
/// weight within the remaining files; a file sending no weight is also a sink,
/// and one receiving none a source. Each bucket is an ordered set, so every
/// choice falls to the lowest index in O(log V). Every entry carries the
/// file's weights, so the chosen file comes with them.
struct Buckets {
    /// The weights of each remaining file.
    remaining: BTreeMap<usize, Weights>,
    /// Each remaining file by its delta; the file is reversed, so the last
    /// entry is the lowest file of the largest delta.
    by_delta: BTreeSet<(i128, Reverse<usize>, Weights)>,
    /// The remaining files that send nothing.
    sinks: BTreeSet<(usize, Weights)>,
    /// The remaining files that receive nothing.
    sources: BTreeSet<(usize, Weights)>,
}

impl Buckets {
    fn new(graph: &Adjacency) -> Self {
        let total = |arcs: &[Arc]| {
            arcs.iter()
                .fold(0_u64, |sum, arc| sum.saturating_add(arc.weight))
        };
        let mut buckets = Self {
            remaining: BTreeMap::new(),
            by_delta: BTreeSet::new(),
            sinks: BTreeSet::new(),
            sources: BTreeSet::new(),
        };
        for (file, (outgoing, incoming)) in graph.outgoing.iter().zip(&graph.incoming).enumerate() {
            let weights = Weights {
                sent: total(outgoing),
                received: total(incoming),
            };
            buckets.remaining.insert(file, weights);
            buckets.file_in(file, weights);
        }
        buckets
    }

    /// Files `file` under `weights`.
    fn file_in(&mut self, file: usize, weights: Weights) {
        self.by_delta
            .insert((weights.delta(), Reverse(file), weights));
        if weights.sent == 0 {
            self.sinks.insert((file, weights));
        }
        if weights.received == 0 {
            self.sources.insert((file, weights));
        }
    }

    /// Takes `file`, filed under `weights`, out of every bucket it sits in.
    fn file_out(&mut self, file: usize, weights: Weights) {
        self.by_delta
            .remove(&(weights.delta(), Reverse(file), weights));
        self.sinks.remove(&(file, weights));
        self.sources.remove(&(file, weights));
    }

    /// The next file and its weights: the lowest sink, else the lowest
    /// source, else the lowest file of the largest outgoing-minus-incoming
    /// weight. `true` marks a sink, which goes to the back of the order.
    fn next(&self) -> Option<(usize, Weights, bool)> {
        if let Some(&(sink, weights)) = self.sinks.first() {
            return Some((sink, weights, true));
        }
        if let Some(&(source, weights)) = self.sources.first() {
            return Some((source, weights, false));
        }
        self.by_delta
            .last()
            .map(|&(_, Reverse(file), weights)| (file, weights, false))
    }

    /// Places `file`, filed under `weights`, withdrawing its weight from its
    /// remaining neighbours.
    fn place(&mut self, graph: &Adjacency, file: usize, weights: Weights) {
        self.remaining.remove(&file);
        self.file_out(file, weights);
        for arc in graph.outgoing.get(file).into_iter().flatten() {
            self.withdraw(arc.file, arc.weight, false);
        }
        for arc in graph.incoming.get(file).into_iter().flatten() {
            self.withdraw(arc.file, arc.weight, true);
        }
    }

    /// Removes `weight` from what `file` sends or receives, unless it is
    /// already placed.
    fn withdraw(&mut self, file: usize, weight: u64, sent: bool) {
        let Some(weights) = self.remaining.get_mut(&file) else {
            return;
        };
        let before = *weights;
        if sent {
            weights.sent = weights.sent.saturating_sub(weight);
        } else {
            weights.received = weights.received.saturating_sub(weight);
        }
        let after = *weights;
        self.file_out(file, before);
        self.file_in(file, after);
    }
}

/// An Eades–Lin–Smyth order: sinks to the back, sources to the front, and
/// otherwise the file with the largest outgoing-minus-incoming weight next,
/// each choice falling to the lowest index. Each placement updates only the
/// placed file's neighbours, so the order costs O((V+E)·log V).
fn greedy_order(graph: &Adjacency) -> Vec<usize> {
    let mut buckets = Buckets::new(graph);
    let mut front: Vec<usize> = Vec::new();
    let mut back: Vec<usize> = Vec::new();
    while let Some((file, weights, sink)) = buckets.next() {
        buckets.place(graph, file, weights);
        if sink {
            back.push(file);
        } else {
            front.push(file);
        }
    }
    front.extend(back.into_iter().rev());
    front
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Adjacency, Walk};

    #[test]
    fn should_still_reach_a_file_after_the_walk_stamp_wraps() {
        // one import 0 -> 1, kept.
        let weights: BTreeMap<(usize, usize), u64> = [((0, 1), 1)].into_iter().collect();
        let graph = Adjacency::new(2, &weights);
        // stale marks from earlier walks equal the stamp a wrapped walk would
        // reuse without the reset: 0 if the stamp stayed at zero, 1 if the
        // marks were not cleared.
        let stale = |mark: u32| Walk {
            visited: vec![mark; 2],
            stamp: u32::MAX,
            stack: Vec::new(),
        };

        for mark in [0, 1] {
            let mut walk = stale(mark);

            let reached = walk.reaches(&graph, &[false], 0, 1);

            assert!(reached, "stale mark {mark}");
            assert_eq!(walk.stamp, 1);
        }
    }
}

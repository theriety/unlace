//! Heuristic minimum feedback arc set solving for large or timed-out SCCs.
//!
//! Builds an Eades-Lin-Smyth vertex ordering, improves it by single-vertex
//! repositioning, and cuts the backward edges of the final ordering.

use super::{BreakSet, EdgeRef, EdgeWeights, SccView};

/// The Eades–Lin–Smyth heuristic with single-vertex local improvement.
///
/// A linear ordering is built by repeatedly stripping sinks (appended on the
/// right) and sources (prepended on the left); when neither exists, the vertex
/// maximising out-degree minus in-degree is removed next. The cut is the set of
/// backward edges in the final ordering, which is then improved by repositioning
/// single vertices while the total cut weight strictly drops.
pub(super) fn solve_heuristic(scc: &SccView, weights: &EdgeWeights) -> BreakSet {
    let order = eades_lin_smyth(scc);
    let order = improve(scc, weights, order);
    let mut edges = backward_edges(scc, &order);
    edges.sort_unstable();
    BreakSet {
        edges,
        exact: false,
    }
}

/// Computes an Eades–Lin–Smyth vertex ordering of the SCC in `O(V + E)`.
fn eades_lin_smyth(scc: &SccView) -> Vec<u32> {
    let vertex_count = scc.vertex_count as usize;
    let mut out_degree = vec![0_i64; vertex_count];
    let mut in_degree = vec![0_i64; vertex_count];
    let mut alive = vec![true; vertex_count];
    for edge in &scc.edges {
        if let Some(slot) = out_degree.get_mut(edge.source as usize) {
            *slot += 1;
        }
        if let Some(slot) = in_degree.get_mut(edge.target as usize) {
            *slot += 1;
        }
    }

    let forward = adjacency(scc, Orientation::Outgoing);
    let backward = adjacency(scc, Orientation::Incoming);

    let mut left: Vec<u32> = Vec::new();
    let mut right: Vec<u32> = Vec::new();

    loop {
        // strip sinks (no live out-edges) to the right.
        if let Some(vertex) = pick(&alive, &out_degree, Degree::Zero) {
            right.push(vertex);
            remove(
                vertex,
                &mut alive,
                &forward,
                &backward,
                &mut out_degree,
                &mut in_degree,
            );
            continue;
        }
        // strip sources (no live in-edges) to the left.
        if let Some(vertex) = pick(&alive, &in_degree, Degree::Zero) {
            left.push(vertex);
            remove(
                vertex,
                &mut alive,
                &forward,
                &backward,
                &mut out_degree,
                &mut in_degree,
            );
            continue;
        }
        // otherwise take the vertex of maximum out-degree minus in-degree,
        // breaking ties by smallest index for determinism. No live vertex
        // left means the ordering is complete.
        let Some(vertex) = pick_max_delta(&alive, &out_degree, &in_degree) else {
            break;
        };
        left.push(vertex);
        remove(
            vertex,
            &mut alive,
            &forward,
            &backward,
            &mut out_degree,
            &mut in_degree,
        );
    }

    right.reverse();
    left.extend(right);
    left
}

/// Which incident edges an adjacency list records.
#[derive(Clone, Copy)]
enum Orientation {
    /// Successors: `vertex -> neighbour`.
    Outgoing,
    /// Predecessors: `neighbour -> vertex`.
    Incoming,
}

/// Builds a per-vertex neighbour list under the given orientation.
fn adjacency(scc: &SccView, orientation: Orientation) -> Vec<Vec<u32>> {
    let mut lists = vec![Vec::new(); scc.vertex_count as usize];
    for edge in &scc.edges {
        let (owner, neighbour) = match orientation {
            Orientation::Outgoing => (edge.source, edge.target),
            Orientation::Incoming => (edge.target, edge.source),
        };
        if let Some(list) = lists.get_mut(owner as usize) {
            list.push(neighbour);
        }
    }
    lists
}

/// A degree predicate used when scanning for the next removable vertex.
#[derive(Clone, Copy)]
enum Degree {
    /// Matches a vertex whose tracked degree has fallen to zero.
    Zero,
}

/// Returns the lowest-indexed live vertex whose `degree` entry satisfies
/// `predicate`, or `None`.
fn pick(alive: &[bool], degree: &[i64], predicate: Degree) -> Option<u32> {
    alive.iter().enumerate().find_map(|(index, &is_alive)| {
        let value = degree.get(index).copied().unwrap_or(0);
        let matches = match predicate {
            Degree::Zero => value == 0,
        };
        if is_alive && matches {
            u32::try_from(index).ok()
        } else {
            None
        }
    })
}

/// Returns the live vertex maximising `out_degree - in_degree`, breaking ties by
/// smallest index, or `None` when no vertex is live.
fn pick_max_delta(alive: &[bool], out_degree: &[i64], in_degree: &[i64]) -> Option<u32> {
    let mut best: Option<(i64, u32)> = None;
    for (vertex, &is_alive) in (0_u32..).zip(alive) {
        if !is_alive {
            continue;
        }
        let index = vertex as usize;
        let delta = out_degree.get(index).copied().unwrap_or(0)
            - in_degree.get(index).copied().unwrap_or(0);
        if best.is_none_or(|(best_delta, _)| delta > best_delta) {
            best = Some((delta, vertex));
        }
    }
    best.map(|(_, vertex)| vertex)
}

/// Removes `vertex` from the live set and decrements the live degree of every
/// neighbour that still references it.
fn remove(
    vertex: u32,
    alive: &mut [bool],
    forward: &[Vec<u32>],
    backward: &[Vec<u32>],
    out_degree: &mut [i64],
    in_degree: &mut [i64],
) {
    alive
        .get_mut(vertex as usize)
        .into_iter()
        .for_each(|slot| *slot = false);
    for &successor in forward.get(vertex as usize).into_iter().flatten() {
        if alive.get(successor as usize).copied().unwrap_or(false)
            && let Some(slot) = in_degree.get_mut(successor as usize)
        {
            *slot -= 1;
        }
    }
    for &predecessor in backward.get(vertex as usize).into_iter().flatten() {
        if alive.get(predecessor as usize).copied().unwrap_or(false)
            && let Some(slot) = out_degree.get_mut(predecessor as usize)
        {
            *slot -= 1;
        }
    }
}

/// Improves `order` by repositioning single vertices, accepting any move that
/// strictly lowers the total backward-edge weight; repeats until no move helps.
fn improve(scc: &SccView, weights: &EdgeWeights, mut order: Vec<u32>) -> Vec<u32> {
    let mut current = backward_weight(scc, weights, &order);
    loop {
        let mut improved = false;
        for from in 0..order.len() {
            for to in 0..order.len() {
                if from == to {
                    continue;
                }
                let candidate = reposition(&order, from, to);
                let weight = backward_weight(scc, weights, &candidate);
                if weight + f64::EPSILON < current {
                    order = candidate;
                    current = weight;
                    improved = true;
                }
            }
        }
        if !improved {
            return order;
        }
    }
}

/// Returns a copy of `order` with the vertex at index `from` removed and
/// reinserted at index `to`.
fn reposition(order: &[u32], from: usize, to: usize) -> Vec<u32> {
    let mut next: Vec<u32> = order
        .iter()
        .enumerate()
        .filter(|&(index, _)| index != from)
        .map(|(_, &vertex)| vertex)
        .collect();
    let insert_at = to.min(next.len());
    next.splice(insert_at..insert_at, order.get(from).copied());
    next
}

/// Sums the cut weight of the backward edges induced by `order`.
fn backward_weight(scc: &SccView, weights: &EdgeWeights, order: &[u32]) -> f64 {
    backward_edges(scc, order)
        .into_iter()
        .map(|edge| weights.weight(edge))
        .sum()
}

/// Returns the edges that point backward in `order`, i.e. whose target precedes
/// their source; cutting exactly these makes the ordering a valid topological
/// order of the residual graph.
fn backward_edges(scc: &SccView, order: &[u32]) -> Vec<EdgeRef> {
    let mut position = vec![0_usize; scc.vertex_count as usize];
    for (rank, &vertex) in order.iter().enumerate() {
        position
            .get_mut(vertex as usize)
            .into_iter()
            .for_each(|slot| *slot = rank);
    }

    scc.edges
        .iter()
        .filter(|edge| {
            let source = position.get(edge.source as usize).copied().unwrap_or(0);
            let target = position.get(edge.target as usize).copied().unwrap_or(0);
            target < source
        })
        .copied()
        .collect()
}

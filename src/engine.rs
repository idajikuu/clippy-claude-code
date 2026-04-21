use crate::pack::{Edge, Pack};
use crate::state::ClaudeCodeState;

/// Port of state-engine.ts. An edge's conditions must all hold for it to fire.
/// Empty conditions always match (the loop fallback).
fn eval_conditions(conds: &[crate::pack::EdgeCondition], input: &ClaudeCodeState) -> bool {
    conds.iter().all(|c| {
        let Some(lhs) = input.get(&c.input) else { return false };
        let Some(rhs) = c.value.as_bool() else { return false };
        match c.op.as_str() {
            "==" => lhs == rhs,
            "!=" => lhs != rhs,
            _ => false,
        }
    })
}

/// Pick the most-specific transition (non-loop) edge firing for the given
/// input. Ties broken by condition count descending, matching the TS impl.
pub fn pick_transition<'a>(
    pack: &'a Pack,
    from_node_id: &str,
    input: &ClaudeCodeState,
) -> Option<&'a Edge> {
    let mut candidates: Vec<&Edge> = pack
        .edges
        .iter()
        .filter(|e| !e.is_loop && e.source == from_node_id && eval_conditions(&e.conditions, input))
        .collect();
    candidates.sort_by(|a, b| b.conditions.len().cmp(&a.conditions.len()));
    candidates.into_iter().next()
}

pub fn loop_for<'a>(pack: &'a Pack, node_id: &str) -> Option<&'a Edge> {
    pack.edges.iter().find(|e| e.is_loop && e.source == node_id)
}

pub fn initial_edge(pack: &Pack) -> Option<&Edge> {
    loop_for(pack, &pack.initial_node)
}

use std::collections::HashSet;

use super::indexer::{PathIndexSnapshot, rank_matches};
use super::model::TreeState;

pub fn apply_query(tree: &mut TreeState, snapshot: &PathIndexSnapshot, query: &str) {
    let matched_node_ids = rank_matches(snapshot, query)
        .into_iter()
        .filter_map(|rel_path| tree.path_to_id.get(&rel_path).copied())
        .collect::<Vec<_>>();
    tree.apply_filtered_matches(&matched_node_ids);
}

pub fn collect_ancestor_ids(
    tree: &TreeState,
    matched_node_ids: &[super::model::NodeId],
) -> HashSet<super::model::NodeId> {
    let mut allowed = HashSet::new();
    allowed.insert(tree.root_id);
    for matched_id in matched_node_ids {
        let mut current = Some(*matched_id);
        while let Some(node_id) = current {
            if !allowed.insert(node_id) {
                break;
            }
            current = tree.node(node_id).and_then(|node| node.parent);
        }
    }
    allowed
}

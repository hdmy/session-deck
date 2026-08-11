use crate::domain::{ConversationTurn, TimelineEvent, TurnActivity};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Lineage from provider records that do not produce a visible timeline event,
/// such as Claude attachments and turn-summary system records.
#[derive(Debug, Clone, Default)]
pub struct LineageNode {
    pub parent_uuid: Option<String>,
    pub logical_parent_uuid: Option<String>,
}

/// A branch path selected from append-only Claude event lineage.
///
/// `path_uuids` contains the visible lineage nodes from root to leaf.  It is
/// kept separate from `events` because provider records such as attachments
/// and turn summaries can bridge two visible nodes without producing an event.
#[derive(Debug, Clone)]
pub struct ResolvedBranch {
    pub leaf_uuid: Option<String>,
    pub root_uuid: Option<String>,
    pub path_uuids: Vec<String>,
    pub events: Vec<TimelineEvent>,
    pub is_active: bool,
}

/// Structural anomalies found while resolving the provider event graph.
///
/// Keep this metadata separate from parsed content so callers can expose a
/// safe diagnostic without re-walking raw provider records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphAnomaly {
    pub code: &'static str,
}

/// Resolves every visible leaf in a Claude transcript into an independent
/// branch path.  The last leaf in source order is active, matching the legacy
/// resolver's selection rule.  Files without UUID lineage retain their source
/// order as one `main` branch for backwards compatibility.
pub fn resolve_branches(
    events: &[TimelineEvent],
    preserved: &HashMap<String, Vec<String>>,
    lineage: &HashMap<String, LineageNode>,
) -> Vec<ResolvedBranch> {
    resolve_branches_with_anomalies(events, preserved, lineage).0
}

/// Resolves branches and reports graph anomalies observed during traversal.
pub fn resolve_branches_with_anomalies(
    events: &[TimelineEvent],
    preserved: &HashMap<String, Vec<String>>,
    lineage: &HashMap<String, LineageNode>,
) -> (Vec<ResolvedBranch>, Vec<GraphAnomaly>) {
    let candidates: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.uuid.is_some() && !event.is_meta && !event.is_sidechain)
        .map(|(index, _)| index)
        .collect();
    if candidates.is_empty() {
        return (
            vec![ResolvedBranch {
                leaf_uuid: None,
                root_uuid: None,
                path_uuids: Vec::new(),
                events: events.to_vec(),
                is_active: true,
            }],
            Vec::new(),
        );
    }

    let uuids: HashSet<&str> = candidates
        .iter()
        .filter_map(|index| events[*index].uuid.as_deref())
        .collect();
    let mut virtual_parents: HashMap<String, String> = HashMap::new();
    for index in &candidates {
        let event = &events[*index];
        if !event.compact_boundary {
            continue;
        }
        let Some(boundary_uuid) = event.uuid.as_deref() else {
            continue;
        };
        let ids = preserved.get(boundary_uuid).cloned().unwrap_or_default();
        for pair in ids.windows(2) {
            virtual_parents.insert(pair[1].clone(), pair[0].clone());
        }
        if let Some(last) = ids.last() {
            virtual_parents.insert(boundary_uuid.to_owned(), last.clone());
        }
    }
    // A UUID may produce multiple visible fragments.  Use its last record as
    // the representative index while preserving first-seen UUID ordering.
    let mut representative = HashMap::<String, usize>::new();
    let mut uuid_order = Vec::new();
    for index in &candidates {
        let Some(uuid) = events[*index].uuid.as_ref() else {
            continue;
        };
        if !representative.contains_key(uuid) {
            uuid_order.push(uuid.clone());
        }
        representative.insert(uuid.clone(), *index);
    }

    let mut has_child = HashSet::new();
    for index in &candidates {
        let parent = events[*index]
            .uuid
            .as_deref()
            .and_then(|uuid| parent_uuid(uuid, events, &virtual_parents, lineage));
        if let Some(parent) = parent {
            if uuids.contains(parent.as_str()) {
                has_child.insert(parent);
            }
        }
    }
    let leaves: Vec<String> = uuid_order
        .iter()
        .filter(|uuid| !has_child.contains(uuid.as_str()))
        .cloned()
        .collect();
    let mut anomalies = Vec::new();
    if has_cycle(&uuid_order, events, &virtual_parents, lineage) {
        anomalies.push(GraphAnomaly {
            code: "conversation_graph_cycle",
        });
    }
    // A cyclic graph has no leaf by definition.  Preserve readable events by
    // selecting the latest visible source-order UUID as a deterministic leaf.
    let branch_leaves = if leaves.is_empty() {
        representative
            .iter()
            .max_by_key(|(_, index)| **index)
            .map(|(uuid, _)| vec![uuid.clone()])
            .unwrap_or_default()
    } else {
        leaves
    };
    let active_leaf = branch_leaves
        .iter()
        .max_by_key(|uuid| representative.get(*uuid).copied().unwrap_or_default())
        .cloned();

    let mut branches = branch_leaves
        .iter()
        .map(|leaf_uuid| {
            let (path_uuids, selected) = select_path(
                leaf_uuid,
                events,
                &candidates,
                &representative,
                preserved,
                &virtual_parents,
                lineage,
            );
            let branch_events = branch_events(events, &selected);
            ResolvedBranch {
                leaf_uuid: Some(leaf_uuid.clone()),
                root_uuid: path_uuids.first().cloned(),
                path_uuids,
                events: branch_events,
                is_active: active_leaf.as_deref() == Some(leaf_uuid.as_str()),
            }
        })
        .collect::<Vec<_>>();
    // Keep the active branch first for consumers that historically read the
    // top-level events/turns, then use source-independent UUID order for the
    // alternate labels.
    branches.sort_by_key(|branch| (!branch.is_active, branch.leaf_uuid.clone()));
    (branches, anomalies)
}

/// Selects the current Claude branch from append-only event lineage.
/// Files without UUID lineage retain their source order for backwards compatibility.
pub fn resolve_main_branch(
    events: &[TimelineEvent],
    preserved: &HashMap<String, Vec<String>>,
    lineage: &HashMap<String, LineageNode>,
) -> Vec<TimelineEvent> {
    resolve_branches(events, preserved, lineage)
        .into_iter()
        .find(|branch| branch.is_active)
        .map(|branch| branch.events)
        .unwrap_or_default()
}

fn select_path(
    leaf_uuid: &str,
    events: &[TimelineEvent],
    candidates: &[usize],
    representative: &HashMap<String, usize>,
    preserved: &HashMap<String, Vec<String>>,
    virtual_parents: &HashMap<String, String>,
    lineage: &HashMap<String, LineageNode>,
) -> (Vec<String>, HashSet<usize>) {
    let mut path_uuids = Vec::new();
    let mut selected = HashSet::new();
    let mut current = Some(leaf_uuid.to_owned());
    let mut visited = HashSet::new();
    while let Some(uuid) = current {
        if !visited.insert(uuid.clone()) {
            break;
        }
        if let Some(index) = representative.get(&uuid) {
            path_uuids.push(uuid.clone());
            selected.insert(*index);
        }
        // Keep walking through non-visible lineage records (attachments and
        // turn summaries) until the root is reached.
        current = parent_uuid(&uuid, events, virtual_parents, lineage);
    }
    path_uuids.reverse();

    // Compaction can preserve earlier messages outside the direct parent chain.
    for index in candidates {
        if events[*index].compact_boundary {
            if let Some(uuid) = events[*index].uuid.as_deref() {
                if selected.contains(index) {
                    if let Some(ids) = preserved.get(uuid) {
                        for preserved_id in ids {
                            if let Some(preserved_index) = representative.get(preserved_id) {
                                selected.insert(*preserved_index);
                            }
                        }
                    }
                }
            }
        }
    }
    (path_uuids, selected)
}

fn has_cycle(
    uuid_order: &[String],
    events: &[TimelineEvent],
    virtual_parents: &HashMap<String, String>,
    lineage: &HashMap<String, LineageNode>,
) -> bool {
    uuid_order.iter().any(|start| {
        let mut visited = HashSet::new();
        let mut current = Some(start.clone());
        while let Some(uuid) = current {
            if !visited.insert(uuid.clone()) {
                return true;
            }
            current = parent_uuid(&uuid, events, virtual_parents, lineage);
        }
        false
    })
}

fn branch_events(events: &[TimelineEvent], selected: &HashSet<usize>) -> Vec<TimelineEvent> {
    let selected_uuids: HashSet<String> = selected
        .iter()
        .filter_map(|index| events[*index].uuid.clone())
        .collect();
    let selected_message_ids: HashSet<String> = selected
        .iter()
        .filter_map(|index| {
            (events[*index].kind == "assistant")
                .then(|| events[*index].message_id.clone())
                .flatten()
        })
        .collect();
    let selected_event_uuids = selected_uuids.clone();
    events
        .iter()
        .enumerate()
        .filter(|(index, event)| {
            selected.contains(index)
                || event
                    .uuid
                    .as_ref()
                    .is_some_and(|uuid| selected_uuids.contains(uuid))
                || (event.kind == "assistant"
                    && event
                        .uuid
                        .as_ref()
                        .is_some_and(|uuid| selected_event_uuids.contains(uuid)))
                || (event.kind == "tool_result"
                    && event.parent_uuid.as_ref().is_some_and(|parent| {
                        selected_event_uuids.contains(parent)
                            || selected_message_ids.contains(parent)
                    }))
                || (event.uuid.is_none() && !event.is_meta && !event.is_sidechain)
        })
        .map(|(_, event)| event.clone())
        .collect()
}

fn parent_uuid(
    uuid: &str,
    events: &[TimelineEvent],
    virtual_parents: &HashMap<String, String>,
    lineage: &HashMap<String, LineageNode>,
) -> Option<String> {
    virtual_parents
        .get(uuid)
        .cloned()
        .or_else(|| {
            lineage.get(uuid).and_then(|node| {
                node.parent_uuid
                    .clone()
                    .or_else(|| node.logical_parent_uuid.clone())
            })
        })
        .or_else(|| {
            events
                .iter()
                .find(|event| event.uuid.as_deref() == Some(uuid))
                .and_then(|event| {
                    event
                        .parent_uuid
                        .clone()
                        .or_else(|| event.logical_parent_uuid.clone())
                })
        })
}

pub fn compact_preserved_ids(value: &Value) -> Vec<String> {
    let metadata = value
        .get("compactMetadata")
        .or_else(|| value.get("compact_metadata"));
    let Some(metadata) = metadata else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for key in ["preservedMessages", "preserved_messages"] {
        if let Some(entry) = metadata.get(key) {
            append_compact_ids(entry, &mut ids, &["anchorUuid", "anchor_uuid", "uuids"]);
        }
    }
    for key in ["preservedSegment", "preserved_segment"] {
        if let Some(entry) = metadata.get(key) {
            append_compact_ids(
                entry,
                &mut ids,
                &[
                    "anchorUuid",
                    "anchor_uuid",
                    "headUuid",
                    "head_uuid",
                    "tailUuid",
                    "tail_uuid",
                ],
            );
        }
    }
    ids
}

fn append_compact_ids(value: &Value, output: &mut Vec<String>, keys: &[&str]) {
    if let Value::Object(object) = value {
        for key in keys {
            match object.get(*key) {
                Some(Value::Array(values)) => {
                    output.extend(values.iter().filter_map(Value::as_str).map(str::to_owned))
                }
                Some(Value::String(value)) => output.push(value.to_owned()),
                _ => {}
            }
        }
    } else if let Value::Array(values) = value {
        output.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
    } else if let Value::String(value) = value {
        output.push(value.to_owned());
    }
}

/// Converts the selected provider events into one provider-neutral turn per user prompt.
pub fn assemble_turns(session_id: &str, events: &mut [TimelineEvent]) -> Vec<ConversationTurn> {
    let mut turns = Vec::new();
    let mut current: Option<ConversationTurn> = None;
    let mut next_id = 1_i64;

    for event in events.iter_mut() {
        if event.is_meta || event.is_sidechain || event.kind == "compact_boundary" {
            continue;
        }
        if event.kind == "user" {
            if current.is_some() {
                finish_turn(current.take(), &mut turns);
            }
            current = Some(ConversationTurn {
                id: next_id,
                session_id: session_id.to_owned(),
                user_prompt: Some(event.content.clone()),
                activities: Vec::new(),
                final_response: None,
                timestamp: event.timestamp,
                completed: false,
            });
            next_id += 1;
        } else if current.is_none() {
            current = Some(ConversationTurn {
                id: next_id,
                session_id: session_id.to_owned(),
                ..ConversationTurn::default()
            });
            next_id += 1;
        }

        let turn = current.as_mut().expect("turn created above");
        event.turn_id = Some(turn.id);
        if event.kind != "user" {
            turn.activities.push(TurnActivity {
                event_id: event.id,
                kind: event.kind.clone(),
                role: event.role.clone(),
                content: event.content.clone(),
                timestamp: event.timestamp,
                tool_name: event.tool_name.clone(),
                tool_use_id: event.tool_use_id.clone(),
                parent_tool_use_id: event.parent_tool_use_id.clone(),
                collapsed: event.collapsed,
                final_response: false,
            });
        }
        if event.kind == "assistant"
            && event.role.as_deref() == Some("assistant")
            && !event.content.trim().is_empty()
        {
            if let Some(previous) = turn
                .activities
                .iter_mut()
                .rev()
                .skip(1)
                .find(|activity| activity.kind == "assistant")
            {
                previous.final_response = false;
            }
            event.final_response = true;
            if let Some(activity) = turn.activities.last_mut() {
                activity.final_response = true;
            }
            turn.final_response = Some(event.content.clone());
            turn.completed = true;
        }
    }
    finish_turn(current, &mut turns);
    let final_event_ids: HashSet<i64> = turns
        .iter_mut()
        .flat_map(|turn| {
            let final_event_id = turn
                .activities
                .iter()
                .rev()
                .find(|activity| {
                    activity.kind == "assistant"
                        && activity.role.as_deref() == Some("assistant")
                        && !activity.content.trim().is_empty()
                })
                .map(|activity| activity.event_id);
            for activity in &mut turn.activities {
                activity.final_response = Some(activity.event_id) == final_event_id;
            }
            turn.final_response = final_event_id.and_then(|event_id| {
                turn.activities
                    .iter()
                    .find(|activity| activity.event_id == event_id)
                    .map(|activity| activity.content.clone())
            });
            turn.completed = turn.final_response.is_some();
            final_event_id
        })
        .collect();
    for event in events {
        event.final_response = final_event_ids.contains(&event.id);
    }
    turns
}

fn finish_turn(turn: Option<ConversationTurn>, output: &mut Vec<ConversationTurn>) {
    if let Some(mut turn) = turn {
        if let Some(final_response) = turn.final_response.as_deref() {
            turn.completed = !final_response.trim().is_empty();
        }
        output.push(turn);
    }
}

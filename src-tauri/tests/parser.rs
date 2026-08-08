use context_vault_lib::{
    providers::{claude::parse_session, insights::normalize_path},
    scanner,
};
use std::io::Write;
use tempfile::{tempdir, NamedTempFile};

fn transcript(body: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("create transcript");
    file.write_all(body.as_bytes()).expect("write transcript");
    file
}

#[test]
fn cwd_history_merges_a_b_a_and_continuation_uses_last_cwd() {
    let file = transcript("{\"type\":\"user\",\"cwd\":\"/a\",\"message\":{\"role\":\"user\",\"content\":\"a\"}}\n{\"type\":\"user\",\"cwd\":\"/b\",\"message\":{\"role\":\"user\",\"content\":\"b\"}}\n{\"type\":\"user\",\"cwd\":\"/a\",\"message\":{\"role\":\"user\",\"content\":\"a2\"}}\n");
    let parsed = parse_session(file.path()).unwrap();
    assert_eq!(parsed.summary.cwd.as_deref(), Some("/a"));
    assert_eq!(parsed.cwd_history.len(), 2);
    assert!(parsed
        .cwd_history
        .iter()
        .any(|cwd| cwd.cwd == "/a" && cwd.last_sequence > 1));
}

#[test]
fn filename_and_transcript_session_id_mismatch_is_partial_safe_diagnostic() {
    let root = tempdir().expect("create transcript root");
    let path = root
        .path()
        .join("123e4567-e89b-12d3-a456-426614174000.jsonl");
    std::fs::write(
        &path,
        "{\"type\":\"user\",\"sessionId\":\"123e4567-e89b-12d3-a456-426614174001\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
    )
    .expect("write transcript");
    let parsed = parse_session(&path).expect("parse transcript");
    assert!(parsed.summary.partial);
    assert!(parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "source_session_id_mismatch"));
}

#[test]
fn extracts_metadata_and_separates_human_prompts_from_tool_results() {
    let file = transcript(
        r#"{"type":"user","sessionId":"session-123","cwd":"/work/birds","gitBranch":"main","timestamp":"2026-01-01T08:00:00Z","message":{"role":"user","content":"Deploy CloudBase"}}
{"type":"assistant","sessionId":"session-123","timestamp":"2026-01-01T08:01:00Z","message":{"role":"assistant","model":"claude-sonnet","content":[{"type":"thinking","thinking":"Inspect the build"},{"type":"text","text":"I will inspect it."},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"npm test"}}]}}
{"type":"user","sessionId":"session-123","timestamp":"2026-01-01T08:02:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"tests passed"}]}}
{"type":"user","sessionId":"session-123","gitBranch":"fix/cloud","timestamp":"2026-01-01T08:03:00Z","message":{"role":"user","content":[{"type":"text","text":"Now inspect the logs"}]}}
{"type":"user","sessionId":"session-123","isMeta":true,"message":{"role":"user","content":"metadata prompt"}}
{"type":"user","sessionId":"session-123","isCompactSummary":true,"message":{"role":"user","content":"compaction summary"}}
{"type":"last-prompt","sessionId":"session-123","lastPrompt":"Now inspect the logs"}
"#,
    );

    let parsed = parse_session(file.path()).expect("parse transcript");
    assert_eq!(parsed.summary.id, "claude:session-123");
    assert_eq!(parsed.summary.provider_id, "claude");
    assert_eq!(parsed.summary.project_id, "/work/birds");
    assert_eq!(parsed.summary.cwd.as_deref(), Some("/work/birds"));
    assert_eq!(parsed.summary.branch.as_deref(), Some("fix/cloud"));
    assert_eq!(
        parsed.summary.first_prompt.as_deref(),
        Some("Deploy CloudBase")
    );
    assert_eq!(
        parsed.summary.last_prompt.as_deref(),
        Some("Now inspect the logs")
    );
    assert_eq!(parsed.summary.models, ["claude-sonnet"]);
    assert_eq!(parsed.summary.tool_count, 1);
    assert_eq!(
        parsed
            .events
            .iter()
            .filter(|event| event.kind == "user")
            .count(),
        2
    );
    assert!(parsed
        .events
        .iter()
        .any(|event| event.kind == "thinking" && event.collapsed));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.kind == "tool_use" && event.tool_name.as_deref() == Some("Bash")));
    assert!(parsed.events.iter().any(|event| event.kind == "tool_result"
        && event.content == "tests passed"
        && event.tool_name.as_deref() == Some("Bash")));
    assert!(!parsed
        .events
        .iter()
        .any(|event| event.content == "metadata prompt"));
    assert!(!parsed
        .events
        .iter()
        .any(|event| event.content == "compaction summary"));
    assert_eq!(parsed.summary.started_at, Some(1_767_254_400_000));
    assert_eq!(parsed.summary.ended_at, Some(1_767_254_580_000));
}

#[test]
fn custom_title_wins_over_ai_title_and_prompt_fallback() {
    let file = transcript(
        r#"{"type":"user","aiTitle":"Generated title","message":{"role":"user","content":"A meaningful request"}}
{"type":"assistant","customTitle":"Pinned title","message":{"role":"assistant","content":"Done"}}
{"type":"assistant","aiTitle":"Later generated title","message":{"role":"assistant","content":"Still done"}}
"#,
    );

    assert_eq!(
        parse_session(file.path())
            .expect("parse transcript")
            .summary
            .title,
        "Pinned title"
    );

    let fallback = transcript(
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"  A   compact\\n title  \"}}\n",
    );
    assert_eq!(
        parse_session(fallback.path())
            .expect("parse transcript")
            .summary
            .title,
        "A compact title"
    );
}

#[test]
fn title_uses_visible_text_instead_of_claude_command_markup() {
    let file = transcript(
        r#"{"type":"user","aiTitle":"<span>Generated title</span>","message":{"role":"user","content":"A meaningful request"}}
{"type":"assistant","customTitle":"<command-message>Repair <strong>CloudBase</strong> &amp; deploy</command-message>","message":{"role":"assistant","content":"Done"}}
"#,
    );

    assert_eq!(
        parse_session(file.path())
            .expect("parse transcript")
            .summary
            .title,
        "Repair CloudBase & deploy"
    );

    let empty_custom_title = transcript(
        r#"{"type":"user","customTitle":"<command-message></command-message>","aiTitle":"<span>Generated title</span>","message":{"role":"user","content":"Prompt fallback"}}
"#,
    );
    assert_eq!(
        parse_session(empty_custom_title.path())
            .expect("parse transcript")
            .summary
            .title,
        "Generated title"
    );
}

#[test]
fn malformed_lines_and_partial_tail_are_nonfatal() {
    let file = transcript(
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"ok\"}}\nnot-json\n{\"type\":",
    );
    let parsed = parse_session(file.path()).expect("parse transcript");

    assert!(parsed.summary.partial);
    assert!(parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "malformed_json"));
    assert!(parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "partial_json"));
    assert_eq!(parsed.summary.first_prompt.as_deref(), Some("ok"));
}

#[test]
fn valid_final_line_without_newline_is_not_partial() {
    let file =
        transcript("{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"complete\"}}");
    let parsed = parse_session(file.path()).expect("parse transcript");

    assert!(!parsed.summary.partial);
    assert!(parsed.diagnostics.is_empty());
}

#[test]
fn nested_subagent_transcripts_are_not_top_level_sessions() {
    let root = tempfile::tempdir().expect("create root");
    let project = root.path().join("project");
    let subagents = project.join("session-123/subagents");
    std::fs::create_dir_all(&subagents).expect("create session tree");
    std::fs::write(
        project.join("session-123.jsonl"),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"main\"}}\n",
    )
    .expect("write main transcript");
    std::fs::write(
        subagents.join("agent-child.jsonl"),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"child\"}}\n",
    )
    .expect("write child transcript");

    let (sessions, diagnostics, complete) = scanner::scan_root(root.path()).expect("scan root");
    assert!(complete);
    assert!(diagnostics.is_empty());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].summary.provider_id, "claude");
    assert_eq!(sessions[0].summary.first_prompt.as_deref(), Some("main"));
}

#[test]
fn graph_resolver_keeps_latest_branch_and_assistant_fragments() {
    let file = transcript(
        r#"{"type":"user","sessionId":"graph-1","uuid":"u1","timestamp":1,"message":{"role":"user","content":"inspect"}}
{"type":"assistant","sessionId":"graph-1","uuid":"a-old","parentUuid":"u1","timestamp":2,"message":{"role":"assistant","content":[{"type":"text","text":"old branch"}]}}
{"type":"assistant","sessionId":"graph-1","uuid":"a-new","parentUuid":"u1","timestamp":3,"message":{"role":"assistant","content":[{"type":"text","text":"new "}]}}
{"type":"assistant","sessionId":"graph-1","uuid":"a-new","parentUuid":"u1","timestamp":4,"message":{"role":"assistant","content":[{"type":"text","text":"branch"}]}}
{"type":"assistant","sessionId":"graph-1","uuid":"agent","parentUuid":"u1","isSidechain":true,"message":{"role":"assistant","content":"hidden agent"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse transcript");
    assert!(!parsed
        .events
        .iter()
        .any(|event| event.content == "old branch"));
    assert!(!parsed
        .events
        .iter()
        .any(|event| event.content == "hidden agent"));
    assert_eq!(
        parsed
            .turns
            .first()
            .and_then(|turn| turn.final_response.as_deref()),
        Some("branch")
    );
    assert_eq!(
        parsed
            .events
            .iter()
            .filter(|event| event.kind == "assistant")
            .count(),
        2
    );
}

#[test]
fn graph_resolver_walks_through_attachment_and_system_lineage_nodes() {
    let file = transcript(
        r#"{"type":"user","sessionId":"bridged-1","uuid":"u1","message":{"role":"user","content":"first prompt"}}
{"type":"assistant","sessionId":"bridged-1","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":"first reply"}}
{"type":"attachment","sessionId":"bridged-1","uuid":"attachment-1","parentUuid":"a1"}
{"type":"user","sessionId":"bridged-1","uuid":"u2","parentUuid":"attachment-1","message":{"role":"user","content":"second prompt"}}
{"type":"assistant","sessionId":"bridged-1","uuid":"a2","parentUuid":"u2","message":{"role":"assistant","content":"second reply"}}
{"type":"system","sessionId":"bridged-1","uuid":"turn-summary-1","parentUuid":"a2","subtype":"turn_duration"}
{"type":"user","sessionId":"bridged-1","uuid":"u3","parentUuid":"turn-summary-1","message":{"role":"user","content":"third prompt"}}
{"type":"assistant","sessionId":"bridged-1","uuid":"a3","parentUuid":"u3","message":{"role":"assistant","content":"third reply"}}
"#,
    );

    let parsed = parse_session(file.path()).expect("parse transcript");
    assert_eq!(parsed.turns.len(), 3);
    assert_eq!(
        parsed
            .turns
            .iter()
            .filter_map(|turn| turn.user_prompt.as_deref())
            .collect::<Vec<_>>(),
        ["first prompt", "second prompt", "third prompt"]
    );
    assert_eq!(
        parsed
            .turns
            .iter()
            .filter_map(|turn| turn.final_response.as_deref())
            .collect::<Vec<_>>(),
        ["first reply", "second reply", "third reply"]
    );
}

#[test]
fn tool_results_are_correlated_and_final_response_is_last_fragment() {
    let file = transcript(
        r#"{"type":"user","sessionId":"tool-1","uuid":"u1","message":{"role":"user","content":"run"}}
{"type":"assistant","sessionId":"tool-1","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":[{"type":"tool_use","id":"tool-1","name":"Bash","input":{"command":"true"}},{"type":"text","text":"working"}]}}
{"type":"user","sessionId":"tool-1","uuid":"r1","parentUuid":"a1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tool-1","content":"ok"}]}}
{"type":"assistant","sessionId":"tool-1","uuid":"a2","parentUuid":"r1","message":{"role":"assistant","content":"done"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse transcript");
    let result = parsed
        .events
        .iter()
        .find(|event| event.kind == "tool_result")
        .expect("tool result");
    assert_eq!(result.tool_use_id.as_deref(), Some("tool-1"));
    assert_eq!(result.tool_name.as_deref(), Some("Bash"));
    assert_eq!(parsed.turns.len(), 1);
    assert_eq!(parsed.turns[0].final_response.as_deref(), Some("done"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.final_response && event.content == "done"));
}

#[test]
fn compact_metadata_object_rewrites_preserved_parent_chain() {
    let file = transcript(
        r#"{"type":"user","sessionId":"compact-1","uuid":"u0","message":{"role":"user","content":"start"}}
{"type":"assistant","sessionId":"compact-1","uuid":"p1","parentUuid":"u0","message":{"role":"assistant","content":"before"}}
{"type":"assistant","sessionId":"compact-1","uuid":"p2","parentUuid":"p1","message":{"role":"assistant","content":"before compact"}}
{"type":"system","sessionId":"compact-1","uuid":"cb","parentUuid":"p2","subtype":"compact_boundary","compactMetadata":{"preservedMessages":{"uuids":["p1","p2"],"anchorUuid":"u0"}}}
{"type":"assistant","sessionId":"compact-1","uuid":"after","parentUuid":"cb","message":{"role":"assistant","content":"after compact"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse compact transcript");
    for content in ["before", "before compact", "after compact"] {
        assert!(
            parsed.events.iter().any(|event| event.content == content),
            "missing {content}"
        );
    }
    assert!(parsed.events.iter().any(|event| event.compact_boundary));
}

#[test]
fn compact_preserved_segment_and_message_fragments_keep_tool_results() {
    let file = transcript(
        r#"{"type":"user","sessionId":"compact-2","uuid":"u0","message":{"role":"user","content":"start"}}
{"type":"assistant","sessionId":"compact-2","uuid":"f1","parentUuid":"u0","message":{"id":"m1","role":"assistant","content":"fragment one"}}
{"type":"user","sessionId":"compact-2","uuid":"r1","parentUuid":"f1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"result"}]}}
{"type":"assistant","sessionId":"compact-2","uuid":"f2","parentUuid":"u0","message":{"id":"m1","role":"assistant","content":"fragment two"}}
{"type":"system","sessionId":"compact-2","uuid":"cb","parentUuid":"f2","subtype":"compact_boundary","compactMetadata":{"preservedSegment":{"headUuid":"f1","tailUuid":"f2","anchorUuid":"u0"}}}
{"type":"assistant","sessionId":"compact-2","uuid":"after","parentUuid":"cb","message":{"role":"assistant","content":"done"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse compact transcript");
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "fragment one"));
    assert!(parsed.events.iter().any(|event| event.content == "result"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "fragment two"));
}

#[test]
fn multiple_tool_results_and_team_nodes_are_losslessly_correlated() {
    let file = transcript(
        r#"{"type":"user","sessionId":"tools-2","uuid":"u0","message":{"role":"user","content":"run"}}
{"type":"assistant","sessionId":"tools-2","uuid":"a1","parentUuid":"u0","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"One","input":{}},{"type":"tool_use","id":"t2","name":"Two","input":{}}]}}
{"type":"user","sessionId":"tools-2","uuid":"r1","parentUuid":"a1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"one"},{"type":"tool_result","tool_use_id":"t2","content":"two"}]}}
{"type":"assistant","sessionId":"tools-2","uuid":"team","parentUuid":"a1","teamName":"workers","message":{"role":"assistant","content":"hidden team"}}
{"type":"assistant","sessionId":"tools-2","uuid":"a2","parentUuid":"r1","message":{"role":"assistant","content":"done"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse tools transcript");
    let results = parsed
        .events
        .iter()
        .filter(|event| event.kind == "tool_result")
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].tool_use_id.as_deref(), Some("t1"));
    assert_eq!(results[1].tool_use_id.as_deref(), Some("t2"));
    assert!(!parsed
        .events
        .iter()
        .any(|event| event.content == "hidden team"));
    assert!(parsed.turns[0]
        .activities
        .iter()
        .all(|activity| activity.kind != "user"));
}

#[test]
fn null_agent_and_team_metadata_do_not_hide_main_conversation() {
    let file = transcript(
        r#"{"type":"user","sessionId":"null-meta","uuid":"u1","agentId":null,"teamName":null,"compactMetadata":null,"message":{"role":"user","content":"keep this prompt"}}
{"type":"assistant","sessionId":"null-meta","uuid":"a1","parentUuid":"u1","agent_id":null,"team_name":null,"message":{"role":"assistant","content":"keep this reply"}}
"#,
    );

    let parsed = parse_session(file.path()).expect("parse nullable metadata");
    assert_eq!(parsed.turns.len(), 1);
    assert_eq!(
        parsed.turns[0].user_prompt.as_deref(),
        Some("keep this prompt")
    );
    assert_eq!(
        parsed.turns[0].final_response.as_deref(),
        Some("keep this reply")
    );
    assert!(parsed.events.iter().all(|event| !event.is_sidechain));
    assert!(parsed.events.iter().all(|event| !event.compact_boundary));
}

#[test]
fn parses_all_leaf_branches_and_preserves_legacy_active_view() {
    let file = transcript(
        r#"{"type":"user","sessionId":"branches-1","uuid":"root","timestamp":1,"message":{"role":"user","content":"choose"}}
{"type":"assistant","sessionId":"branches-1","uuid":"old-leaf","parentUuid":"root","timestamp":2,"message":{"role":"assistant","content":"old reply"}}
{"type":"assistant","sessionId":"branches-1","uuid":"new-leaf","parentUuid":"root","timestamp":3,"message":{"role":"assistant","content":"new reply"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse branches");

    assert_eq!(parsed.branches.len(), 2);
    assert_eq!(parsed.branches[0].summary.id, "leaf:new-leaf");
    assert_eq!(parsed.branches[0].summary.label, "main");
    assert!(parsed.branches[0].summary.is_active);
    assert_eq!(parsed.branches[1].summary.id, "leaf:old-leaf");
    assert_eq!(parsed.branches[1].summary.label, "alternate-1");
    assert_eq!(
        parsed.branches[1].summary.fork_point_uuid.as_deref(),
        Some("root")
    );
    assert!(parsed.branches[1]
        .events
        .iter()
        .any(|event| event.content == "old reply"));
    assert!(!parsed
        .events
        .iter()
        .any(|event| event.content == "old reply"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "new reply"));
    assert_eq!(parsed.turns.len(), 1);
    assert_eq!(parsed.turns[0].final_response.as_deref(), Some("new reply"));
}

#[test]
fn uuid_less_transcript_has_one_stable_main_branch() {
    let file = transcript(
        r#"{"type":"user","sessionId":"no-branch","message":{"role":"user","content":"prompt"}}
{"type":"assistant","sessionId":"no-branch","message":{"role":"assistant","content":"reply"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse no-uuid transcript");

    assert_eq!(parsed.branches.len(), 1);
    let summary = &parsed.branches[0].summary;
    assert_eq!(summary.id, "main");
    assert_eq!(summary.label, "main");
    assert_eq!(summary.kind, "main");
    assert!(summary.leaf_uuid.is_none());
    assert!(summary.is_active);
    assert_eq!(parsed.events.len(), parsed.branches[0].events.len());
    assert_eq!(parsed.turns.len(), parsed.branches[0].turns.len());
}

#[test]
fn derives_tool_and_file_insights_without_exposing_inputs() {
    let file = transcript(
        r#"{"type":"user","sessionId":"insights-1","cwd":"/work","message":{"role":"user","content":"update files"}}
{"type":"assistant","sessionId":"insights-1","message":{"role":"assistant","content":[{"type":"tool_use","id":"edit-1","name":"Edit","input":{"file_path":"/work/src/main.rs","old_string":"old line","new_string":"new line\nsecond"}},{"type":"tool_use","id":"write-1","name":"Write","input":{"path":"/tmp/readme.md","content":"outside"}},{"type":"tool_use","id":"multi-1","name":"MultiEdit","input":{"file_path":"src/lib.rs","edits":[{"old_string":"a","new_string":"b\nc"},{"old_string":"d\ne","new_string":"f"}]}},{"type":"tool_use","id":"notebook-1","name":"NotebookEdit","input":{"notebook_path":"/work/notebook.ipynb","new_source":"print(1)"}},{"type":"tool_use","id":"bash-1","name":"Bash","input":{"command":"echo no file inference"}},{"type":"tool_use","id":"unknown-1","name":"Mystery","input":{"file_path":"src/not-a-change"}},{"type":"tool_use","id":"bad-path-1","name":"Edit","input":{"file_path":"../escape","old_string":"x","new_string":"y"}},{"type":"tool_use","id":"control-path-1","name":"Write","input":{"file_path":"bad\u0007name","content":"hidden"}}]}}
{"type":"user","sessionId":"insights-1","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"edit-1","content":"ok","is_error":false},{"type":"tool_result","tool_use_id":"write-1","content":"failed","is_error":true},{"type":"tool_result","tool_use_id":"multi-1","content":"unknown"},{"type":"tool_result","tool_use_id":"notebook-1","content":"ok","is_error":false},{"type":"tool_result","tool_use_id":"bash-1","content":"ok","is_error":false},{"type":"tool_result","tool_use_id":"unknown-1","content":"unknown"},{"type":"tool_result","tool_use_id":"bad-path-1","content":"ok","is_error":false},{"type":"tool_result","tool_use_id":"control-path-1","content":"ok","is_error":false}]}}
{"type":"assistant","sessionId":"insights-1","message":{"role":"assistant","content":[{"type":"text","text":"done"}]}}
"#,
    );
    let before = std::fs::read(file.path()).expect("read source before parse");
    let parsed = parse_session(file.path()).expect("parse insights transcript");
    let after = std::fs::read(file.path()).expect("read source after parse");
    assert_eq!(before, after);

    let branch = &parsed.branches[0];
    let changes = &branch.turn_insights[0].file_changes;
    assert!(changes.iter().any(|change| {
        change.path == "src/main.rs"
            && change.kind == "modified"
            && change.additions == 2
            && change.deletions == 1
    }));
    assert!(changes
        .iter()
        .any(|change| change.path == "outside-workspace/readme.md" && change.kind == "written"));
    assert!(changes.iter().any(|change| change.path == "src/lib.rs"
        && change.additions == 3
        && change.deletions == 3));
    assert!(changes
        .iter()
        .any(|change| change.path == "notebook.ipynb" && change.kind == "notebook"));
    assert!(!changes.iter().any(|change| change.path.contains("escape")));
    assert!(!changes.iter().any(|change| change.path.contains("bad")));
    assert!(!serde_json::to_string(changes)
        .expect("serialize summaries")
        .contains("new line"));
    assert!(!parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code.contains("old line")));

    let stat = |name: &str| {
        branch
            .tool_stats
            .iter()
            .find(|stat| stat.name == name)
            .expect("tool stat")
    };
    assert_eq!(stat("Edit").successes, 2);
    assert_eq!(stat("Write").failures, 1);
    assert_eq!(stat("MultiEdit").unknown, 1);
    assert_eq!(stat("Bash").files_changed, 0);
    assert_eq!(stat("Mystery").files_changed, 0);
}

#[test]
fn cyclic_graphs_are_bounded_diagnostic_and_keep_readable_events() {
    let two_node = transcript(
        r#"{"type":"user","sessionId":"cycle-2","uuid":"u1","parentUuid":"a1","message":{"role":"user","content":"cycle prompt"}}
{"type":"assistant","sessionId":"cycle-2","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":"cycle reply"}}
"#,
    );
    let parsed = parse_session(two_node.path()).expect("parse two-node cycle");
    assert!(parsed.summary.partial);
    assert!(parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "conversation_graph_cycle"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "cycle prompt"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "cycle reply"));
    assert!(parsed
        .branches
        .iter()
        .any(|branch| branch.summary.is_active));

    let self_cycle = transcript(
        r#"{"type":"user","sessionId":"cycle-self","uuid":"self","parentUuid":"self","message":{"role":"user","content":"self prompt"}}
{"type":"assistant","sessionId":"cycle-self","uuid":"reply","parentUuid":"self","message":{"role":"assistant","content":"self reply"}}
"#,
    );
    let parsed = parse_session(self_cycle.path()).expect("parse self cycle");
    assert!(parsed.summary.partial);
    assert!(parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "conversation_graph_cycle"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "self reply"));

    let cycle_with_leaf = transcript(
        r#"{"type":"user","sessionId":"cycle-leaf","uuid":"u1","parentUuid":"a1","message":{"role":"user","content":"cycle root"}}
{"type":"assistant","sessionId":"cycle-leaf","uuid":"a1","parentUuid":"u1","message":{"role":"assistant","content":"cycle edge"}}
{"type":"assistant","sessionId":"cycle-leaf","uuid":"leaf","parentUuid":"u1","message":{"role":"assistant","content":"normal leaf"}}
"#,
    );
    let parsed = parse_session(cycle_with_leaf.path()).expect("parse cycle with leaf");
    assert!(parsed.summary.partial);
    assert!(parsed
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "conversation_graph_cycle"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "cycle edge"));
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "normal leaf"));
    assert_eq!(
        parsed
            .branches
            .first()
            .and_then(|branch| branch.summary.leaf_uuid.as_deref()),
        Some("leaf")
    );
}

#[test]
fn assistant_fragments_with_colliding_message_ids_stay_on_selected_branch() {
    let file = transcript(
        r#"{"type":"user","sessionId":"collision-1","uuid":"root","message":{"role":"user","content":"choose"}}
{"type":"assistant","sessionId":"collision-1","uuid":"old","parentUuid":"root","message":{"id":"shared","role":"assistant","content":"old branch"}}
{"type":"assistant","sessionId":"collision-1","uuid":"new","parentUuid":"root","message":{"id":"shared","role":"assistant","content":"new branch"}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse collision transcript");
    assert!(parsed
        .events
        .iter()
        .any(|event| event.content == "new branch"));
    assert!(!parsed
        .events
        .iter()
        .any(|event| event.content == "old branch"));
}

#[test]
fn tool_result_status_and_name_stay_isolated_per_branch() {
    let file = transcript(
        r#"{"type":"user","sessionId":"branch-tools","uuid":"root","message":{"role":"user","content":"choose"}}
{"type":"assistant","sessionId":"branch-tools","uuid":"old-use","parentUuid":"root","message":{"role":"assistant","content":[{"type":"tool_use","id":"same-id","name":"OldTool","input":{}}]}}
{"type":"user","sessionId":"branch-tools","uuid":"old-result","parentUuid":"old-use","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"same-id","is_error":true,"content":"old failure"}]}}
{"type":"assistant","sessionId":"branch-tools","uuid":"new-use","parentUuid":"root","message":{"role":"assistant","content":[{"type":"tool_use","id":"same-id","name":"NewTool","input":{}}]}}
{"type":"user","sessionId":"branch-tools","uuid":"new-result","parentUuid":"new-use","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"same-id","is_error":false,"content":"new success"}]}}
"#,
    );
    let parsed = parse_session(file.path()).expect("parse branch tool transcript");
    let old = parsed
        .branches
        .iter()
        .find(|branch| branch.summary.leaf_uuid.as_deref() == Some("old-result"))
        .expect("old branch");
    let new = parsed
        .branches
        .iter()
        .find(|branch| branch.summary.leaf_uuid.as_deref() == Some("new-result"))
        .expect("new branch");
    let old_stat = old
        .tool_stats
        .iter()
        .find(|stat| stat.name == "OldTool")
        .expect("old tool stat");
    let new_stat = new
        .tool_stats
        .iter()
        .find(|stat| stat.name == "NewTool")
        .expect("new tool stat");
    assert_eq!(old_stat.failures, 1);
    assert_eq!(old_stat.successes, 0);
    assert_eq!(new_stat.successes, 1);
    assert_eq!(new_stat.failures, 0);
    assert!(!old.tool_stats.iter().any(|stat| stat.name == "NewTool"));
    assert!(!new.tool_stats.iter().any(|stat| stat.name == "OldTool"));
}

#[test]
fn normalizes_insight_paths_lexically_at_boundaries() {
    assert_eq!(normalize_path("/work", Some("/work")), Some(".".to_owned()));
    assert_eq!(
        normalize_path("/tmp/readme.md", None),
        Some("outside-workspace/readme.md".to_owned())
    );
    assert_eq!(
        normalize_path("/other/readme.md", Some("/work")),
        Some("outside-workspace/readme.md".to_owned())
    );
    assert_eq!(normalize_path("../escape", Some("/work")), None);
    assert_eq!(
        normalize_path("C:\\work\\src\\main.rs\r\n", Some("C:\\work")),
        Some("src/main.rs".to_owned())
    );
}

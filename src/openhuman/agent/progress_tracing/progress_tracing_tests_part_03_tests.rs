use super::*;

#[test]
fn subagent_content_is_withheld_when_capture_off() {
    let mut c = collect(&[
        (AgentProgress::TurnStarted, 0),
        (spawn("task-1", "Researcher"), 5),
        (
            AgentProgress::SubagentCompleted {
                agent_id: "researcher".to_string(),
                task_id: "task-1".to_string(),
                elapsed_ms: 100,
                iterations: 2,
                output_chars: 12,
                output: "final answer".to_string(),
                worktree_path: None,
                changed_files: vec![],
                dirty_status: None,
            },
            105,
        ),
    ]);
    c.finish(110);
    let sub = find(c.spans(), "subagent.Researcher");
    assert!(sub.input.is_none());
    assert!(sub.output.is_none());
}

#[test]
fn oversized_model_content_degrades_to_truncated_string() {
    let big = "x".repeat(MAX_MODEL_CONTENT_CHARS + 100);
    let captured = capture_model_content(&serde_json::json!({ "content": big }));
    let rendered = match &captured {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    assert!(rendered.chars().count() <= MAX_MODEL_CONTENT_CHARS + 64);
    assert!(rendered.contains("truncated"));
}

#[test]
fn turn_content_respects_the_trace_context_capture_gate() {
    // Regression (PR #4506 review): the collector briefly carried TWO capture
    // gates — a collector-level flag (checked by the TurnContent arm) and the
    // TraceContext flag (checked everywhere else). The web progress bridge only
    // sets the TraceContext flag, so TurnContent silently dropped the turn's
    // prompt/reply even with capture_content enabled. There is now a single
    // gate: both construction styles must attach TurnContent.
    for collector in [
        SpanCollector::new(ctx().with_capture_content(true)),
        SpanCollector::new(ctx()).with_content_capture(true),
    ] {
        let mut c = collector;
        c.record(&AgentProgress::TurnStarted, 0);
        c.record(
            &AgentProgress::TurnContent {
                input: Some("the prompt".to_string()),
                output: Some("the reply".to_string()),
            },
            5,
        );
        c.finish(10);
        let turn = find(c.spans(), "agent.turn");
        assert_eq!(
            turn.input,
            Some(serde_json::Value::String("the prompt".to_string()))
        );
        assert_eq!(
            turn.output,
            Some(serde_json::Value::String("the reply".to_string()))
        );
    }
}

// ── turn trace identity (#4496) ─────────────────────────────────────────────

#[test]
fn turn_trace_id_prefers_the_ui_session_then_falls_back_to_the_thread() {
    // The exporter writes this string as the `trace-create` body's `id`, and a
    // feedback score has to name the same one. Both branches are live: a PTT
    // turn carries a UI session id, ordinary chat does not.
    assert_eq!(
        turn_trace_id(Some(42), "thread-abc", "req-1"),
        "42:req-1",
        "a caller-supplied session id wins"
    );
    assert_eq!(
        turn_trace_id(None, "thread-abc", "req-1"),
        "thread-abc:req-1",
        "ordinary chat falls back to the thread id"
    );
}

#[test]
fn turn_trace_id_is_built_from_trace_session_id() {
    // Pins the two against each other so the delivery path and the exporter
    // cannot drift into scoring a trace that was never created.
    for session in [None, Some(7u64)] {
        assert_eq!(
            turn_trace_id(session, "thread-x", "req-9"),
            format!("{}:req-9", trace_session_id(session, "thread-x")),
        );
    }
}

#[test]
fn turn_tracing_is_enabled_by_either_sink() {
    let mut config = crate::openhuman::config::Config::default();

    config.observability.share_usage_data = false;
    config.observability.agent_tracing.enabled = false;
    assert!(
        !turn_tracing_enabled(&config),
        "with neither sink on, no trace is created and none may be stamped"
    );

    config.observability.share_usage_data = true;
    assert!(
        turn_tracing_enabled(&config),
        "usage sharing alone is enough"
    );

    config.observability.share_usage_data = false;
    config.observability.agent_tracing.enabled = true;
    assert!(
        turn_tracing_enabled(&config),
        "an explicit agent_tracing opt-in is enough on its own"
    );
}

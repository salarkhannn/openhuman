use super::*;

/// An unparseable base is the fail-closed default rather than a panic or a
/// pushable bucket. `ingestion_url` returns a non-URL placeholder when no
/// backend host resolves.
#[test]
fn an_unparseable_base_is_production() {
    for base in ["", "not a url", "/api/v1/ingestion"] {
        assert_eq!(
            environment_for_base(base),
            "production",
            "{base:?} must fail closed"
        );
    }
}

#[test]
fn trace_create_carries_environment_release_and_run_tags() {
    let mut turn = span(
        "trace-1",
        "root",
        None,
        "agent.turn",
        SpanKind::Turn,
        SpanStatus::Ok,
        1_000,
        Some(2_000),
    );
    turn.attributes
        .insert("run.type".into(), json!("autonomous_task"));
    turn.attributes
        .insert("channel.source".into(), json!("autonomous"));
    let payload = spans_to_langfuse_batch(&[turn], false, "staging");
    let trace = &payload["batch"][0]["body"];
    // Top-level Langfuse trace fields, not metadata.
    assert_eq!(trace["environment"], "staging");
    assert_eq!(trace["release"], env!("CARGO_PKG_VERSION"));
    // Filterable run tags + run_type metadata.
    assert_eq!(
        trace["tags"],
        json!(["run:autonomous_task", "source:autonomous"])
    );
    assert_eq!(trace["metadata"]["run_type"], "autonomous_task");
}

#[test]
fn interactive_chat_trace_gets_interactive_run_tag() {
    let mut turn = span(
        "trace-1",
        "root",
        None,
        "agent.turn",
        SpanKind::Turn,
        SpanStatus::Ok,
        1_000,
        Some(2_000),
    );
    turn.attributes
        .insert("run.type".into(), json!("interactive_chat"));
    turn.attributes
        .insert("channel.source".into(), json!("chat"));
    let payload = spans_to_langfuse_batch(&[turn], false, "production");
    let trace = &payload["batch"][0]["body"];
    assert_eq!(
        trace["tags"],
        json!(["run:interactive_chat", "source:chat"])
    );
    assert_eq!(trace["metadata"]["run_type"], "interactive_chat");
}

#[test]
fn generation_usage_details_map_reasoning_and_cache_tokens() {
    let mut gen = span(
        "trace-1",
        "gen-1",
        Some("root"),
        "llm.agentic-v1",
        SpanKind::Generation,
        SpanStatus::Ok,
        1_000,
        Some(1_500),
    );
    gen.attributes.clear();
    gen.attributes
        .insert("gen_ai.request.model".into(), json!("agentic-v1"));
    gen.attributes
        .insert("gen_ai.usage.input_tokens".into(), json!(1_000));
    gen.attributes
        .insert("gen_ai.usage.output_tokens".into(), json!(200));
    gen.attributes
        .insert("gen_ai.usage.cached_input_tokens".into(), json!(0));
    gen.attributes
        .insert("gen_ai.usage.reasoning_tokens".into(), json!(128));
    gen.attributes
        .insert("gen_ai.usage.cache_creation_tokens".into(), json!(64));
    gen.attributes
        .insert("gen_ai.usage.cost_usd".into(), json!(0.0042));
    gen.attributes
        .insert("gen_ai.provider".into(), json!("managed"));

    let payload = spans_to_langfuse_batch(&[gen], false, "production");
    let obs = &payload["batch"][1];
    assert_eq!(obs["type"], "generation-create");
    let usage = &obs["body"]["usageDetails"];
    assert_eq!(usage["input"], 1_000);
    assert_eq!(usage["output"], 200);
    // Cache reads always flow, even at 0.
    assert_eq!(usage["cache_read_input_tokens"], 0);
    assert_eq!(usage["reasoning_tokens"], 128);
    assert_eq!(usage["cache_creation_input_tokens"], 64);
    assert_eq!(obs["body"]["costDetails"]["total"], 0.0042);
    // Provenance rides in observation metadata.
    assert_eq!(obs["body"]["metadata"]["gen_ai.provider"], "managed");
}

#[test]
fn generation_without_reasoning_or_cache_write_omits_those_usage_keys() {
    let mut gen = span(
        "trace-1",
        "gen-1",
        Some("root"),
        "llm.agentic-v1",
        SpanKind::Generation,
        SpanStatus::Ok,
        1_000,
        Some(1_500),
    );
    gen.attributes.clear();
    gen.attributes
        .insert("gen_ai.usage.input_tokens".into(), json!(10));
    gen.attributes
        .insert("gen_ai.usage.output_tokens".into(), json!(5));
    let payload = spans_to_langfuse_batch(&[gen], false, "production");
    let usage = &payload["batch"][1]["body"]["usageDetails"];
    assert_eq!(
        usage["cache_read_input_tokens"], 0,
        "cache reads always present"
    );
    assert!(usage.get("reasoning_tokens").is_none());
    assert!(usage.get("cache_creation_input_tokens").is_none());
}

#[test]
fn error_span_gets_error_level_and_status_message() {
    let mut tool = span(
        "trace-1",
        "tool-1",
        Some("root"),
        "tool.shell",
        SpanKind::Tool,
        SpanStatus::Error,
        1_000,
        Some(1_200),
    );
    tool.attributes
        .insert("error.message".into(), json!("The command timed out"));
    let payload = spans_to_langfuse_batch(&[tool], false, "production");
    let obs = &payload["batch"][1]["body"];
    assert_eq!(obs["level"], "ERROR");
    assert_eq!(obs["statusMessage"], "The command timed out");

    // Without a captured message: ERROR level, no statusMessage.
    let bare = span(
        "trace-1",
        "tool-2",
        Some("root"),
        "tool.shell",
        SpanKind::Tool,
        SpanStatus::Error,
        1_000,
        Some(1_200),
    );
    let payload = spans_to_langfuse_batch(&[bare], false, "production");
    let obs = &payload["batch"][1]["body"];
    assert_eq!(obs["level"], "ERROR");
    assert!(obs.get("statusMessage").is_none());
}

#[tokio::test]
async fn empty_spans_push_is_ok_noop() {
    let config = Config::default();
    // Empty batch short-circuits before any host/token resolution or network.
    assert!(push_spans(&config, &[]).await.is_ok());
}

#[test]
fn score_batch_carries_the_trace_id_name_and_value() {
    let batch = build_score_batch("thread-7:req-42", "user-feedback", 1.0, None);
    let events = batch["batch"].as_array().expect("batch array");
    assert_eq!(events.len(), 1, "exactly one score event");

    let event = &events[0];
    assert_eq!(event["type"], "score-create");
    assert!(event["id"].as_str().is_some(), "envelope id present");
    assert!(
        event["timestamp"].as_str().unwrap().contains('T'),
        "ISO timestamp"
    );

    let body = &event["body"];
    assert_eq!(
        body["traceId"], "thread-7:req-42",
        "the score must name the turn's trace or Langfuse orphans it"
    );
    assert_eq!(body["name"], "user-feedback");
    assert_eq!(body["value"], 1.0);
    assert!(body["id"].as_str().is_some(), "score id present");
    assert_ne!(
        body["id"], event["id"],
        "the score's own id is distinct from the envelope's"
    );
    assert!(
        body.get("comment").is_none(),
        "comment is omitted entirely rather than sent as null"
    );
}

#[test]
fn score_batch_keeps_a_thumbs_down_at_zero() {
    // A truthiness bug here would silently drop every negative rating, which is
    // the half of the signal worth having.
    let batch = build_score_batch("trace-1", "user-feedback", 0.0, None);
    let body = &batch["batch"][0]["body"];
    assert_eq!(body["value"], 0.0);
    assert!(body["value"].is_number(), "value stays numeric, not a bool");
}

#[test]
fn score_batch_includes_a_comment_when_given() {
    let batch = build_score_batch("trace-1", "quality", 0.5, Some("partially answered"));
    assert_eq!(batch["batch"][0]["body"]["comment"], "partially answered");
}

#[tokio::test]
async fn push_score_is_a_noop_when_usage_sharing_is_off() {
    // The privacy gate is the first thing `push_score` checks, so this returns
    // before the environment check, the session lookup and the request. The
    // config is otherwise identical to the one used by the failing test below,
    // which is what proves the gate — and nothing else — is what shortened this
    // path: flip `share_usage_data` and the same inputs produce an error.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.observability.share_usage_data = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config.config_path = dir.path().join("config.toml");

    assert!(
        push_score(&config, "trace-1", "user-feedback", 1.0, None)
            .await
            .is_ok(),
        "opting out of telemetry is a success, not a failure"
    );
}

#[tokio::test]
async fn push_score_reports_a_refused_push_rather_than_swallowing_it() {
    // Gate on, a local origin (which resolves to the `development` environment
    // and IS push-allowed, so `skip_push` does not short-circuit), and a
    // credential root with no stored session. The push is therefore attempted
    // and refused at the session check, before any network call.
    //
    // `config_path` is what makes this hermetic: credential state resolves
    // against its parent, so pointing it at a tempdir is what stops the test
    // reading the developer's real profile and, with a live session, posting a
    // real score.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.observability.share_usage_data = true;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config.config_path = dir.path().join("config.toml");

    let err = push_score(&config, "trace-1", "user-feedback", 1.0, None)
        .await
        .expect_err("a push with no session must not report success");
    assert!(
        err.contains("no backend session token"),
        "unexpected error: {err}"
    );
}

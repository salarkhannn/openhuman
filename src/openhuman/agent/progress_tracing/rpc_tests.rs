use super::*;
use serde_json::json;

fn request(trace_id: &str) -> SubmitScoreRequest {
    SubmitScoreRequest {
        trace_id: trace_id.to_string(),
        name: "user-feedback".to_string(),
        value: 1.0,
        comment: None,
    }
}

/// A hermetic config with telemetry opted out.
///
/// Three things matter, and each was a defect in an earlier revision of this
/// feature. `share_usage_data = false` is the gate under test. The local
/// `api_url` resolves to the `development` environment, which IS push-allowed,
/// so the gate is the only thing that can shorten the path — flip it and the
/// same config produces an error. And `config_path` points at a tempdir
/// because credential state resolves against its parent: without it this test
/// reads the developer's real `~/.openhuman` profile and, with a live session,
/// posts a real score to Langfuse from a unit test.
fn opted_out_config(dir: &tempfile::TempDir) -> Config {
    let mut config = Config::default();
    config.observability.share_usage_data = false;
    config.api_url = Some("http://127.0.0.1:9".to_string());
    config.config_path = dir.path().join("config.toml");
    config
}

#[test]
fn schema_describes_the_submit_score_surface() {
    let schemas = all_controller_schemas();
    assert_eq!(schemas.len(), 1);

    let schema = &schemas[0];
    assert_eq!(schema.namespace, "observability");
    assert_eq!(schema.function, "submit_score");

    let inputs: Vec<_> = schema.inputs.iter().map(|f| (f.name, f.required)).collect();
    assert_eq!(
        inputs,
        vec![
            ("trace_id", true),
            ("name", true),
            ("value", true),
            ("comment", false),
        ]
    );
    assert_eq!(schema.inputs[2].ty, TypeSchema::F64);

    // `ok` is a real outcome, so `error` has to be part of the declared surface
    // for a caller to be able to act on a failure.
    let outputs: Vec<_> = schema
        .outputs
        .iter()
        .map(|f| (f.name, f.required))
        .collect();
    assert_eq!(outputs, vec![("ok", true), ("error", false)]);
    assert_eq!(schema.outputs[0].ty, TypeSchema::Bool);
}

#[test]
fn the_rpc_method_name_is_the_one_the_renderer_calls() {
    let controllers = all_internal_controllers();
    assert_eq!(controllers.len(), 1);
    assert_eq!(
        controllers[0].rpc_method_name(),
        "openhuman.observability_submit_score"
    );
}

#[test]
fn submit_score_is_absent_from_the_agent_facing_registry() {
    // The security property this module exists to hold: an agent that could
    // call this would be scoring its own output. It is registered internal-only
    // (`build_internal_only_controllers`), so it must not appear in the
    // agent-facing controller set.
    let agent_facing: Vec<String> = crate::core::all::all_registered_controllers()
        .iter()
        .map(|c| c.rpc_method_name())
        .collect();
    assert!(
        !agent_facing
            .iter()
            .any(|m| m == "openhuman.observability_submit_score"),
        "observability_submit_score must stay out of the agent-facing registry"
    );
}

#[test]
fn params_deserialize_with_and_without_a_comment() {
    let bare: SubmitScoreRequest = serde_json::from_value(json!({
        "trace_id": "t:req-1",
        "name": "user-feedback",
        "value": 0.0,
    }))
    .expect("comment is optional");
    assert_eq!(bare.trace_id, "t:req-1");
    assert_eq!(bare.value, 0.0);
    assert_eq!(bare.comment, None);

    let with_comment: SubmitScoreRequest = serde_json::from_value(json!({
        "trace_id": "t:req-1",
        "name": "quality",
        "value": 0.5,
        "comment": "partially answered",
    }))
    .expect("comment accepted");
    assert_eq!(with_comment.comment.as_deref(), Some("partially answered"));
}

#[test]
fn params_missing_a_required_field_are_rejected() {
    let err = serde_json::from_value::<SubmitScoreRequest>(json!({ "trace_id": "t:req-1" }))
        .expect_err("name and value are required");
    assert!(err.to_string().contains("name"), "unexpected error: {err}");
}

#[tokio::test]
async fn a_withheld_score_reports_success() {
    // Declining to send telemetry is not a failure, so the UI must not show the
    // user's rating as having failed when they opted out of usage sharing.
    let dir = tempfile::tempdir().expect("tempdir");
    let response = submit_score_with(&opted_out_config(&dir), request("t:req-1")).await;
    assert!(response.ok);
    assert_eq!(response.error, None);
}

#[tokio::test]
async fn a_failed_push_reports_the_failure_instead_of_a_bare_ok() {
    // The regression this guards: returning a hardcoded `ok: true` made the UI
    // confirm a rating that never reached Langfuse, and made the whole RPC
    // untestable — the assertion would pass with the push removed entirely.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = opted_out_config(&dir);
    config.observability.share_usage_data = true;

    let response = submit_score_with(&config, request("t:req-1")).await;
    assert!(!response.ok, "a rejected push must not report ok");
    let error = response.error.expect("a failure carries its reason");
    assert!(
        error.contains("no backend session token"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn the_response_serializes_error_only_when_present() {
    let ok = serde_json::to_value(SubmitScoreResponse {
        ok: true,
        error: None,
    })
    .unwrap();
    assert_eq!(ok, json!({ "ok": true }), "no null error on the happy path");

    let failed = serde_json::to_value(SubmitScoreResponse {
        ok: false,
        error: Some("nope".to_string()),
    })
    .unwrap();
    assert_eq!(failed, json!({ "ok": false, "error": "nope" }));
}

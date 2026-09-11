//! Renderer-facing RPC for attaching a feedback score to a turn's trace.
//!
//! The chat UI offers thumbs up/down on an assistant reply; each click submits
//! one Langfuse `score-create` against the trace that produced that reply. The
//! trace id is not guessed by the renderer — it is read back from the message's
//! own `extraMetadata.traceId`, which the core stamped when it persisted the
//! reply (`web_chat::reply_persistence`). See [`super::turn_trace_id`] for the
//! single derivation both sides share.
//!
//! **Registered internal-only.** `src/core/all.rs` pushes this into
//! `build_internal_only_controllers`, never the agent-facing registry, so the
//! method is reachable from the renderer but absent from the tool catalogue an
//! agent can call. Feedback is a statement about the agent's output; letting
//! the agent score itself would make the signal worthless.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;

use super::langfuse;

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;

/// Parameters for `openhuman.observability_submit_score`.
#[derive(Debug, Deserialize)]
pub struct SubmitScoreRequest {
    /// Trace the score attaches to, as stamped on the rated message.
    pub trace_id: String,
    /// Metric name, e.g. `user-feedback`.
    pub name: String,
    /// Numeric score value (1.0 / 0.0 for the thumbs).
    pub value: f64,
    /// Optional free-text explanation.
    #[serde(default)]
    pub comment: Option<String>,
}

/// Outcome of a submission.
///
/// `ok` is the real result, not a constant: a swallowed failure would leave the
/// UI showing a confirmed rating for a score that never reached Langfuse. The
/// RPC itself still never fails — feedback must not raise an error dialog over
/// a chat reply — so a rejected push comes back as `ok: false` plus `error`.
#[derive(Debug, Serialize)]
pub struct SubmitScoreResponse {
    /// Whether the score reached Langfuse (also true when the privacy gate
    /// withheld it — declining to send telemetry is not a failure).
    pub ok: bool,
    /// Why the push failed. Absent when `ok` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "submit_score" => submit_score_schema(),
        other => panic!("unknown observability controller schema `{other}`"),
    }
}

fn submit_score_schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "observability",
        function: "submit_score",
        description: "Attach a feedback or quality score to an agent turn's trace.",
        inputs: vec![
            FieldSchema {
                name: "trace_id",
                comment:
                    "Trace to attach the score to, from the message's `extraMetadata.traceId`.",
                ty: TypeSchema::String,
                required: true,
            },
            FieldSchema {
                name: "name",
                comment: "Metric name, e.g. 'user-feedback' or 'triage-quality'.",
                ty: TypeSchema::String,
                required: true,
            },
            FieldSchema {
                name: "value",
                comment: "Numeric score value; 1.0 and 0.0 for a thumbs up/down.",
                ty: TypeSchema::F64,
                required: true,
            },
            FieldSchema {
                name: "comment",
                comment: "Optional free-text explanation of the score.",
                ty: TypeSchema::String,
                required: false,
            },
        ],
        outputs: vec![
            FieldSchema {
                name: "ok",
                comment: "True when the score reached Langfuse or the privacy gate withheld it.",
                ty: TypeSchema::Bool,
                required: true,
            },
            FieldSchema {
                name: "error",
                comment: "Why the submission failed; absent when `ok` is true.",
                ty: TypeSchema::String,
                required: false,
            },
        ],
    }
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("submit_score")]
}

/// Internal-only registration. Deliberately NOT exposed through
/// `build_registered_controllers` — see the module docs.
pub fn all_internal_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("submit_score"),
        handler: handle_submit_score,
    }]
}

/// Submit one score against `config`.
///
/// The config-free seam [`handle_submit_score`] wraps: it lets the behaviour be
/// tested against an in-memory `Config` instead of whatever `~/.openhuman`
/// happens to hold, so a unit test can exercise the privacy gate without
/// touching the developer's real profile or reaching the network.
pub(crate) async fn submit_score_with(
    config: &Config,
    request: SubmitScoreRequest,
) -> SubmitScoreResponse {
    match langfuse::push_score(
        config,
        &request.trace_id,
        &request.name,
        request.value,
        request.comment.as_deref(),
    )
    .await
    {
        Ok(()) => {
            log::debug!(
                "[observability] submit_score ok trace_id={} name={} value={}",
                request.trace_id,
                request.name,
                request.value
            );
            SubmitScoreResponse {
                ok: true,
                error: None,
            }
        }
        Err(err) => {
            // Non-fatal by design: the turn is long finished and the user's
            // rating is not worth an error dialog. Logged so a silently
            // dropped score is still diagnosable from the core log.
            log::warn!(
                "[observability] submit_score failed trace_id={} name={} error={}",
                request.trace_id,
                request.name,
                err
            );
            SubmitScoreResponse {
                ok: false,
                error: Some(err),
            }
        }
    }
}

fn handle_submit_score(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!("[observability] handle_submit_score enter params={params:?}");
        let request = match serde_json::from_value::<SubmitScoreRequest>(Value::Object(params)) {
            Ok(request) => request,
            Err(err) => {
                log::warn!("[observability] handle_submit_score invalid params error={err}");
                return Err(format!("invalid params: {err}"));
            }
        };
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(config) => config,
            Err(err) => {
                log::warn!("[observability] handle_submit_score config load failed error={err}");
                return Err(err);
            }
        };
        let response = submit_score_with(&config, request).await;
        serde_json::to_value(response).map_err(|err| format!("serialize response failed: {err}"))
    })
}

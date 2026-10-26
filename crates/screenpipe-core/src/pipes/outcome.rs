// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Deciding whether a finished pipe run did its job.
//!
//! A zero exit code is not enough on its own. The agent (Pi in JSON mode)
//! reports every message it produces on stdout, so a run is only a success
//! when that stream ends in a final answer that finished normally. Everything
//! else is a failure with a category the UI and the history can show:
//!
//! - `no_final_result`: the agent exited cleanly but its last answer is
//!   missing, was cut off at the output limit, was aborted, or is empty;
//! - `context_compaction`: compacting the context failed before a final
//!   answer arrived (Pi reports `auto_compaction_end` with an error);
//! - `content_filter`: the provider's content filter stopped the response;
//! - `refusal`: the model or provider refused (a `refusal` stop or finish
//!   reason, a safety refusal, or a cybersecurity-risk flag);
//! - `ai_preset_unavailable`: the run asked for a provider that is not
//!   configured on this machine;
//! - and the older text-matched classes (`rate_limited`, `auth_failed`,
//!   `model_not_found`, `network`, `context_overflow`).
//!
//! Refusal and content-filter markers are only looked for in error texts
//! (stderr, provider error messages and stop reasons), never in what the
//! assistant wrote as its answer.

use serde_json::Value;
use std::collections::HashSet;

/// `error_type` for a clean exit without a verified final answer.
pub const ERROR_NO_FINAL_RESULT: &str = "no_final_result";
/// `error_type` for a context compaction that failed before the answer.
pub const ERROR_CONTEXT_COMPACTION: &str = "context_compaction";
/// `error_type` for a response blocked by the provider's content filter.
pub const ERROR_CONTENT_FILTER: &str = "content_filter";
/// `error_type` for a model or provider refusal.
pub const ERROR_REFUSAL: &str = "refusal";
/// `error_type` for a run whose AI provider is not configured.
pub const ERROR_AI_PRESET_UNAVAILABLE: &str = "ai_preset_unavailable";
/// `error_type` (from the base) for a context that outgrew the model.
pub const ERROR_CONTEXT_OVERFLOW: &str = "context_overflow";

/// Stop reasons that mean the model finished its answer on its own.
const NORMAL_STOPS: &[&str] = &["stop", "end_turn", "endturn", "stop_sequence"];

/// How a finished run is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    pub success: bool,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
}

impl RunOutcome {
    fn ok() -> Self {
        Self {
            success: true,
            error_type: None,
            error_message: None,
        }
    }

    fn failed(error_type: Option<&str>, message: impl Into<String>) -> Self {
        Self {
            success: false,
            error_type: error_type.map(str::to_string),
            error_message: Some(message.into()),
        }
    }

    /// The execution status to store: `completed` or `failed`.
    pub fn status(&self) -> &'static str {
        if self.success {
            "completed"
        } else {
            "failed"
        }
    }
}

/// The last assistant message of a run, as the agent stream reported it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FinalMessage {
    pub stop_reason: Option<String>,
    pub text: String,
    pub error: Option<String>,
}

/// What the agent's JSON event stream says about a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamDigest {
    /// At least one line was an agent event (a JSON object with `type`).
    pub recognized: bool,
    /// The agent reported the end of its loop (`agent_end`).
    pub agent_ended: bool,
    /// Last assistant message seen.
    pub final_message: Option<FinalMessage>,
    /// Distinct tool calls the agent made.
    pub tool_calls: usize,
    /// A compaction ended at least once.
    pub compaction_seen: bool,
    /// Error of the latest compaction, when it failed.
    pub compaction_failure: Option<String>,
    /// Error texts and stop reasons reported by the provider.
    pub provider_errors: Vec<String>,
}

impl StreamDigest {
    /// The stream ends in an answer the model finished on its own.
    pub fn has_verified_result(&self) -> bool {
        let Some(last) = &self.final_message else {
            return false;
        };
        if last.error.is_some() {
            return false;
        }
        let normal = last
            .stop_reason
            .as_deref()
            .map(|r| NORMAL_STOPS.contains(&r.to_ascii_lowercase().as_str()))
            .unwrap_or(false);
        normal && (!last.text.trim().is_empty() || self.tool_calls > 0)
    }
}

/// Read the agent's stdout. Lines that are not JSON objects are ignored, so
/// mixed output still works; a stream with no agent events at all is
/// reported as not recognised.
pub fn digest_stream(stdout: &str) -> StreamDigest {
    let mut digest = StreamDigest::default();
    let mut tool_ids: HashSet<String> = HashSet::new();
    let mut anonymous_tools = 0usize;

    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') || !line.contains("\"type\"") {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(kind) = event.get("type").and_then(Value::as_str) else {
            continue;
        };
        digest.recognized = true;

        match kind {
            "message_end" | "turn_end" => {
                if let Some(message) = event.get("message") {
                    note_message(&mut digest, message, &mut tool_ids, &mut anonymous_tools);
                }
            }
            "agent_end" => {
                digest.agent_ended = true;
                if let Some(messages) = event.get("messages").and_then(Value::as_array) {
                    for message in messages {
                        note_message(&mut digest, message, &mut tool_ids, &mut anonymous_tools);
                    }
                }
            }
            "tool_execution_start" => {
                match event
                    .get("toolCallId")
                    .or_else(|| event.get("tool_call_id"))
                    .and_then(Value::as_str)
                {
                    Some(id) => {
                        tool_ids.insert(id.to_string());
                    }
                    None => anonymous_tools += 1,
                }
            }
            other if other.ends_with("compaction_end") => {
                digest.compaction_seen = true;
                let error = event
                    .get("errorMessage")
                    .or_else(|| event.get("error"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|e| !e.is_empty())
                    .map(str::to_string);
                let aborted = event.get("aborted").and_then(Value::as_bool) == Some(true);
                digest.compaction_failure = match (error, aborted) {
                    (Some(e), _) => Some(e),
                    (None, true) => Some("compaction was aborted".to_string()),
                    (None, false) => None,
                };
            }
            _ => {}
        }
    }

    digest.tool_calls = tool_ids.len() + anonymous_tools;
    digest
}

fn note_message(
    digest: &mut StreamDigest,
    message: &Value,
    tool_ids: &mut HashSet<String>,
    anonymous_tools: &mut usize,
) {
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let stop_reason = ["stopReason", "stop_reason", "finish_reason", "finishReason"]
        .iter()
        .find_map(|k| message.get(*k).and_then(Value::as_str))
        .map(str::to_string);
    let error = message
        .get("errorMessage")
        .or_else(|| message.get("error_message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_string);

    let mut text = String::new();
    match message.get("content") {
        Some(Value::String(s)) => text.push_str(s),
        Some(Value::Array(blocks)) => {
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                    }
                    Some("toolCall") | Some("tool_use") | Some("tool_call") => {
                        match block.get("id").and_then(Value::as_str) {
                            Some(id) => {
                                tool_ids.insert(id.to_string());
                            }
                            None => *anonymous_tools += 1,
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    if let Some(e) = &error {
        digest.provider_errors.push(e.clone());
    }
    if let Some(reason) = &stop_reason {
        let lower = reason.to_ascii_lowercase();
        if !NORMAL_STOPS.contains(&lower.as_str()) && lower != "tooluse" && lower != "tool_use" {
            digest
                .provider_errors
                .push(format!("stop reason: {reason}"));
        }
    }
    digest.final_message = Some(FinalMessage {
        stop_reason,
        text,
        error,
    });
}

/// Decide how a run that returned normally is recorded. `exit_ok` is the
/// executor's own verdict (exit code and provider errors it noticed).
pub fn assess_run(exit_ok: bool, stdout: &str, stderr: &str) -> RunOutcome {
    let digest = digest_stream(stdout);
    let verified = if digest.recognized {
        digest.has_verified_result()
    } else {
        !stdout.trim().is_empty()
    };
    if exit_ok && verified {
        return RunOutcome::ok();
    }

    let mut error_texts: Vec<&str> = Vec::new();
    if !stderr.trim().is_empty() {
        error_texts.push(stderr);
    }
    for e in &digest.provider_errors {
        error_texts.push(e);
    }

    if let Some(failure) = digest.compaction_failure.as_deref() {
        // Pi reports a failed overflow recovery through the same event; it
        // keeps the older class, which also starts the next run fresh.
        if failure.to_lowercase().contains("overflow") {
            return RunOutcome::failed(
                Some(ERROR_CONTEXT_OVERFLOW),
                format!("the context overflowed and could not be recovered: {failure}"),
            );
        }
        return RunOutcome::failed(
            Some(ERROR_CONTEXT_COMPACTION),
            format!("context compaction failed before a final answer: {failure}"),
        );
    }
    if error_texts.iter().any(|t| mentions_compaction_failure(t)) {
        return RunOutcome::failed(
            Some(ERROR_CONTEXT_COMPACTION),
            "context compaction failed before a final answer",
        );
    }
    match refusal_kind(&error_texts) {
        Some(Refusal::ContentFilter) => {
            return RunOutcome::failed(
                Some(ERROR_CONTENT_FILTER),
                "the provider's content filter blocked the response",
            )
        }
        Some(Refusal::Refused) => {
            return RunOutcome::failed(Some(ERROR_REFUSAL), "the model refused to do the task")
        }
        None => {}
    }
    if error_texts.iter().any(|t| mentions_missing_provider(t)) {
        return RunOutcome::failed(
            Some(ERROR_AI_PRESET_UNAVAILABLE),
            "the AI provider this pipe uses is not configured on this machine",
        );
    }

    if !exit_ok {
        let filtered = super::filter_ndjson_stdout(stdout);
        let (error_type, error_message) = super::parse_error_type_from_output(stderr, &filtered);
        return RunOutcome {
            success: false,
            error_type,
            error_message,
        };
    }

    RunOutcome::failed(Some(ERROR_NO_FINAL_RESULT), missing_result_reason(&digest))
}

fn missing_result_reason(digest: &StreamDigest) -> String {
    if !digest.recognized {
        return "the agent exited without printing a result".to_string();
    }
    match &digest.final_message {
        None => "the agent exited without a final answer".to_string(),
        Some(last) => match last.stop_reason.as_deref().map(str::to_ascii_lowercase) {
            Some(r) if r == "length" || r == "max_tokens" => {
                "the final answer was cut off at the output limit".to_string()
            }
            Some(r) if r == "aborted" => "the agent's final answer was aborted".to_string(),
            Some(r) if r == "tooluse" || r == "tool_use" => {
                "the agent exited while still waiting on a tool call".to_string()
            }
            Some(r) if NORMAL_STOPS.contains(&r.as_str()) => {
                "the agent finished without producing an answer".to_string()
            }
            Some(r) => format!("the agent stopped without a final answer (stop reason: {r})"),
            None => "the agent exited without a finished final answer".to_string(),
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    ContentFilter,
    Refused,
}

fn refusal_kind(texts: &[&str]) -> Option<Refusal> {
    let mut refused = false;
    for text in texts {
        let lower = text.to_lowercase();
        let compact: String = lower
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
            .collect();
        if lower.contains("content_filter")
            || lower.contains("content filter")
            || lower.contains("content_policy")
            || lower.contains("content policy")
        {
            return Some(Refusal::ContentFilter);
        }
        let reason_refusal = ["stop_reason", "stopreason", "finish_reason", "finishreason"]
            .iter()
            .any(|k| {
                compact.contains(&format!("{k}:refusal"))
                    || compact.contains(&format!("{k}=refusal"))
            })
            || compact.contains("stopreason:refus");
        let safety_refusal = lower.contains("safety")
            && (lower.contains("refus") || lower.contains("declin") || lower.contains("blocked"));
        let cyber_flag =
            lower.contains("cyber") && (lower.contains("flag") || lower.contains("risk"));
        if reason_refusal
            || safety_refusal
            || cyber_flag
            || lower.contains("refusal")
            || lower.contains("refused to")
        {
            refused = true;
        }
    }
    refused.then_some(Refusal::Refused)
}

fn mentions_compaction_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("compaction")
        && (lower.contains("failed")
            || lower.contains("rejected")
            || lower.contains("stayed above"))
}

fn mentions_missing_provider(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("no ai gateway is configured")
        || lower.contains("no ai preset")
        || lower.contains("preset not found")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_end(text: &str, stop: &str) -> String {
        serde_json::json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": text}],
                "stopReason": stop,
            }
        })
        .to_string()
    }

    fn tool_call_end(id: &str) -> String {
        serde_json::json!({
            "type": "message_end",
            "message": {
                "role": "assistant",
                "content": [{"type": "toolCall", "id": id, "name": "bash", "arguments": {}}],
                "stopReason": "toolUse",
            }
        })
        .to_string()
    }

    fn agent_end(text: &str, stop: &str) -> String {
        serde_json::json!({
            "type": "agent_end",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "go"}]},
                {"role": "assistant", "content": [{"type": "text", "text": text}], "stopReason": stop},
            ]
        })
        .to_string()
    }

    #[test]
    fn a_finished_answer_is_a_success() {
        let stdout = [
            r#"{"type":"agent_start"}"#.to_string(),
            tool_call_end("t1"),
            assistant_end("Done: wrote the digest.", "stop"),
            agent_end("Done: wrote the digest.", "stop"),
        ]
        .join("\n");
        let outcome = assess_run(true, &stdout, "");
        assert_eq!(outcome, RunOutcome::ok());
        assert_eq!(outcome.status(), "completed");
    }

    #[test]
    fn clean_exit_without_a_final_answer_fails() {
        // The process exited 0 after a tool call, but no answer followed.
        let stdout = [r#"{"type":"agent_start"}"#.to_string(), tool_call_end("t1")].join("\n");
        let outcome = assess_run(true, &stdout, "");
        assert!(!outcome.success);
        assert_eq!(outcome.status(), "failed");
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_NO_FINAL_RESULT));
        assert!(outcome
            .error_message
            .unwrap()
            .contains("waiting on a tool call"));
    }

    #[test]
    fn clean_exit_with_no_output_at_all_fails() {
        let outcome = assess_run(true, "", "");
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_NO_FINAL_RESULT));
        let outcome = assess_run(true, r#"{"type":"agent_start"}"#, "");
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_NO_FINAL_RESULT));
    }

    #[test]
    fn a_truncated_or_empty_answer_is_not_verified() {
        let cut = assistant_end("The summary is", "length");
        let outcome = assess_run(true, &cut, "");
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_NO_FINAL_RESULT));
        assert!(outcome.error_message.unwrap().contains("output limit"));

        let empty = assistant_end("   ", "stop");
        let outcome = assess_run(true, &empty, "");
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_NO_FINAL_RESULT));

        // An empty closing message after real tool work still counts.
        let stdout = [tool_call_end("t9"), assistant_end("", "stop")].join("\n");
        assert!(assess_run(true, &stdout, "").success);
    }

    #[test]
    fn plain_text_output_is_judged_by_its_presence() {
        assert!(assess_run(true, "All done.\n", "").success);
        assert!(!assess_run(true, "  \n", "").success);
    }

    #[test]
    fn compaction_failure_gets_its_own_class() {
        let stdout = [
            tool_call_end("t1"),
            r#"{"type":"auto_compaction_start","reason":"threshold"}"#.to_string(),
            r#"{"type":"auto_compaction_end","aborted":false,"willRetry":false,"errorMessage":"the history summary was rejected"}"#.to_string(),
        ]
        .join("\n");
        let digest = digest_stream(&stdout);
        assert!(digest.compaction_seen);
        assert!(digest.compaction_failure.is_some());
        for exit_ok in [true, false] {
            let outcome = assess_run(exit_ok, &stdout, "");
            assert_eq!(
                outcome.error_type.as_deref(),
                Some(ERROR_CONTEXT_COMPACTION)
            );
        }

        // Reported only through the provider error text.
        let errored = serde_json::json!({
            "type": "message_end",
            "message": {"role": "assistant", "content": [], "stopReason": "error",
                "errorMessage": "Context compaction failed during this run (summary too short)."}
        })
        .to_string();
        let outcome = assess_run(false, &errored, "");
        assert_eq!(
            outcome.error_type.as_deref(),
            Some(ERROR_CONTEXT_COMPACTION)
        );
    }

    #[test]
    fn pi_compaction_end_events_are_read() {
        // Pi 0.84 names the event `compaction_end`; a skipped compaction has
        // no result and no error and is not a failure.
        let skipped = [
            r#"{"type":"compaction_end","reason":"threshold","aborted":false,"willRetry":false}"#
                .to_string(),
            assistant_end("Done.", "stop"),
        ]
        .join("\n");
        assert!(assess_run(true, &skipped, "").success);

        let failed = r#"{"type":"compaction_end","reason":"threshold","aborted":false,"willRetry":false,"errorMessage":"Compaction failed: summary too short"}"#;
        assert_eq!(
            assess_run(true, failed, "").error_type.as_deref(),
            Some(ERROR_CONTEXT_COMPACTION)
        );

        let overflow = r#"{"type":"compaction_end","reason":"overflow","aborted":false,"willRetry":false,"errorMessage":"Context overflow recovery failed after one compact-and-retry attempt."}"#;
        assert_eq!(
            assess_run(false, overflow, "").error_type.as_deref(),
            Some(ERROR_CONTEXT_OVERFLOW)
        );
    }

    #[test]
    fn a_successful_compaction_does_not_fail_the_run() {
        let stdout = [
            r#"{"type":"auto_compaction_end","aborted":false,"willRetry":false,"result":{"summary":"x"}}"#.to_string(),
            assistant_end("Finished.", "stop"),
        ]
        .join("\n");
        assert!(assess_run(true, &stdout, "").success);
    }

    #[test]
    fn refusals_and_content_filters_are_classified() {
        let filtered = serde_json::json!({
            "type": "message_end",
            "message": {"role": "assistant", "content": [], "stopReason": "error",
                "errorMessage": "Provider finish_reason: content_filter"}
        })
        .to_string();
        let outcome = assess_run(
            false,
            &filtered,
            "LLM error: Provider finish_reason: content_filter",
        );
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_CONTENT_FILTER));

        let refused = serde_json::json!({
            "type": "message_end",
            "message": {"role": "assistant", "content": [], "stopReason": "refusal"}
        })
        .to_string();
        let outcome = assess_run(true, &refused, "");
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_REFUSAL));

        let outcome = assess_run(false, "", r#"{"stop_reason": "refusal"}"#);
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_REFUSAL));

        let outcome = assess_run(
            false,
            "",
            "request flagged for potential cybersecurity risk by the provider",
        );
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_REFUSAL));

        let outcome = assess_run(false, "", "response declined for safety reasons");
        assert_eq!(outcome.error_type.as_deref(), Some(ERROR_REFUSAL));
    }

    #[test]
    fn refusal_words_in_the_answer_itself_do_not_count() {
        let stdout = assistant_end(
            "The team discussed the content filter and a refusal from the vendor.",
            "stop",
        );
        assert!(assess_run(true, &stdout, "").success);
    }

    #[test]
    fn older_error_classes_still_apply_to_failed_exits() {
        let outcome = assess_run(false, "", "HTTP 429: rate limit exceeded");
        assert_eq!(outcome.error_type.as_deref(), Some("rate_limited"));
        let outcome = assess_run(false, "", "something odd happened");
        assert!(!outcome.success);
        assert_eq!(outcome.error_type, None);
    }

    #[test]
    fn a_missing_gateway_is_a_preset_problem() {
        let outcome = assess_run(
            false,
            "",
            "this preset uses the AI gateway provider (screenpipe-cloud), but no AI gateway is configured.",
        );
        assert_eq!(
            outcome.error_type.as_deref(),
            Some(ERROR_AI_PRESET_UNAVAILABLE)
        );
    }

    #[test]
    fn a_failed_exit_with_a_good_answer_stays_failed() {
        // The executor's own verdict wins: an LLM error it saw mid-run.
        let stdout = assistant_end("Finished.", "stop");
        let outcome = assess_run(false, &stdout, "LLM error: overloaded");
        assert!(!outcome.success);
    }

    #[test]
    fn agent_end_messages_count_as_the_final_answer() {
        let stdout = agent_end("Here is the result.", "stop");
        let digest = digest_stream(&stdout);
        assert!(digest.agent_ended);
        assert!(digest.has_verified_result());
        assert_eq!(
            digest.final_message.unwrap().text,
            "Here is the result.".to_string()
        );
    }

    #[test]
    fn tool_calls_are_counted_once() {
        let call = tool_call_end("same");
        let start = r#"{"type":"tool_execution_start","toolCallId":"same","toolName":"bash"}"#;
        let stdout = [call.clone(), start.to_string(), call].join("\n");
        assert_eq!(digest_stream(&stdout).tool_calls, 1);
    }
}

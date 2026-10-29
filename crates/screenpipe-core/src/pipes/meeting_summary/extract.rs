// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Finding a `## Summary` section in markdown and in a pipe run's output.
//!
//! The meeting-summary pipe prints the summary it is about to save, starting
//! with a `## Summary` line. When the save itself never happened, the
//! finalizer takes that printed section from the run's stdout. Pi writes
//! stdout as JSON events, so the assistant's text is read from the
//! `message_end` / `agent_end` messages; any other output is read as plain
//! text.
//!
//! A section starts at a level-2 heading whose text is "Summary" (any case,
//! with or without a trailing colon) outside fenced code, and runs to the
//! next level-1 or level-2 heading or the end of the text. When a text holds
//! several, the last one wins: it is the one written last.

use serde_json::Value;

/// The body of the last `## Summary` section in `markdown`, without its
/// heading and trimmed. `None` when there is no such section or it is empty.
pub fn summary_section(markdown: &str) -> Option<String> {
    let lines: Vec<&str> = markdown.split('\n').collect();
    let mut start: Option<usize> = None;
    let mut end = lines.len();
    let mut fence: Option<char> = None;

    for (i, line) in lines.iter().enumerate() {
        if let Some(marker) = fence_marker(line) {
            fence = match fence {
                Some(open) if open == marker => None,
                Some(open) => Some(open),
                None => Some(marker),
            };
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let Some((level, text)) = heading(line) else {
            continue;
        };
        if level == 2 && is_summary_title(text) {
            start = Some(i);
            end = lines.len();
        } else if level <= 2 && start.is_some() && end == lines.len() {
            end = i;
        }
    }

    let start = start?;
    let body = lines[start + 1..end].join("\n");
    let body = body.trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// Text of every assistant message in a run's stdout, oldest first. Pi JSON
/// output is read from its message events (the `agent_end` list when there is
/// one, since it is complete); anything else is returned as one plain text.
pub fn assistant_texts(stdout: &str) -> Vec<String> {
    let mut from_messages: Vec<String> = Vec::new();
    let mut from_agent_end: Option<Vec<String>> = None;
    let mut recognized = false;

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
        recognized = true;
        match kind {
            "message_end" => {
                if let Some(text) = event.get("message").and_then(assistant_text) {
                    from_messages.push(text);
                }
            }
            "agent_end" => {
                let texts = event
                    .get("messages")
                    .and_then(Value::as_array)
                    .map(|messages| messages.iter().filter_map(assistant_text).collect())
                    .unwrap_or_default();
                from_agent_end = Some(texts);
            }
            _ => {}
        }
    }

    if let Some(texts) = from_agent_end.filter(|t| !t.is_empty()) {
        return texts;
    }
    if recognized {
        return from_messages;
    }
    let plain = stdout.trim();
    if plain.is_empty() {
        Vec::new()
    } else {
        vec![plain.to_string()]
    }
}

/// The summary section the agent printed last in a run's output.
pub fn summary_from_run_output(stdout: &str) -> Option<String> {
    assistant_texts(stdout)
        .iter()
        .rev()
        .find_map(|text| summary_section(text))
}

fn assistant_text(message: &Value) -> Option<String> {
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let text = match message.get("content")? {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    (!text.trim().is_empty()).then_some(text)
}

/// ATX heading level and text (up to three spaces of indent).
fn heading(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_end_matches('\r');
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let level = rest.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let after = &rest[level..];
    if !after.is_empty() && !after.starts_with([' ', '\t']) {
        return None;
    }
    Some((level, after.trim().trim_end_matches('#').trim()))
}

fn is_summary_title(text: &str) -> bool {
    text.trim_end_matches(':')
        .trim()
        .eq_ignore_ascii_case("summary")
}

fn fence_marker(line: &str) -> Option<char> {
    let t = line.trim_start();
    if t.starts_with("```") {
        Some('`')
    } else if t.starts_with("~~~") {
        Some('~')
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_runs_to_the_next_top_heading() {
        let md = "# Standup\nnotes\n\n## Summary\nWe agreed on the plan.\n### Decisions\n- ship it\n\n## Follow-up\nlater";
        assert_eq!(
            summary_section(md).as_deref(),
            Some("We agreed on the plan.\n### Decisions\n- ship it")
        );
    }

    #[test]
    fn the_last_section_wins_and_case_and_colon_do_not_matter() {
        let md = "## Summary\nold draft\n\n## SUMMARY:\nfinal version";
        assert_eq!(summary_section(md).as_deref(), Some("final version"));
    }

    #[test]
    fn headings_inside_code_fences_are_ignored() {
        let md = "## Summary\nreal\n```\n## Summary\nfake\n```\nmore";
        assert_eq!(
            summary_section(md).as_deref(),
            Some("real\n```\n## Summary\nfake\n```\nmore")
        );
        assert_eq!(summary_section("```\n## Summary\nx\n```"), None);
    }

    #[test]
    fn empty_or_missing_sections_are_none() {
        assert_eq!(summary_section("no headings here"), None);
        assert_eq!(summary_section("## Summary\n   \n## Next"), None);
        assert_eq!(summary_section("### Summary\nlevel three"), None);
        assert_eq!(summary_section("##Summary\nno space"), None);
    }

    fn message_end(text: &str) -> String {
        serde_json::json!({
            "type": "message_end",
            "message": {"role": "assistant", "content": [{"type": "text", "text": text}], "stopReason": "stop"}
        })
        .to_string()
    }

    #[test]
    fn summary_is_taken_from_a_pi_json_stream() {
        // Fixture shaped like a meeting-summary run that printed its summary
        // but never reached the save step.
        let stdout = [
            r#"{"type":"session","id":"s1"}"#.to_string(),
            r#"{"type":"agent_start"}"#.to_string(),
            serde_json::json!({
                "type": "message_end",
                "message": {"role": "assistant", "content": [
                    {"type": "text", "text": "Reading the transcript."},
                    {"type": "toolCall", "id": "c1", "name": "bash", "arguments": {"command": "curl ..."}}
                ], "stopReason": "toolUse"}
            })
            .to_string(),
            serde_json::json!({
                "type": "message_end",
                "message": {"role": "toolResult", "content": [{"type": "text", "text": "## Summary\nnot from the assistant"}]}
            })
            .to_string(),
            message_end("Here it is:\n\n## Summary\nThe team picked Friday for the release.\n\n**Action items**\n- Ana: notes by Thursday"),
        ]
        .join("\n");
        assert_eq!(
            summary_from_run_output(&stdout).as_deref(),
            Some("The team picked Friday for the release.\n\n**Action items**\n- Ana: notes by Thursday")
        );
    }

    #[test]
    fn agent_end_list_is_preferred_and_the_latest_text_wins() {
        let agent_end = serde_json::json!({
            "type": "agent_end",
            "messages": [
                {"role": "user", "content": "summarize"},
                {"role": "assistant", "content": [{"type": "text", "text": "## Summary\nfirst try"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "Saved.\n\n## Summary\nsecond try"}]},
                {"role": "assistant", "content": [{"type": "text", "text": "All done."}]},
            ]
        })
        .to_string();
        let stdout = [message_end("## Summary\nstale"), agent_end].join("\n");
        assert_eq!(
            summary_from_run_output(&stdout).as_deref(),
            Some("second try")
        );
    }

    #[test]
    fn plain_text_output_works_too() {
        let stdout = "thinking...\n## Summary\nShort sync about hiring.\n";
        assert_eq!(
            summary_from_run_output(stdout).as_deref(),
            Some("Short sync about hiring.")
        );
        assert_eq!(summary_from_run_output(""), None);
        assert_eq!(
            summary_from_run_output(&message_end("Nothing was said in this meeting.")),
            None
        );
    }
}

// Handvel — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Handvel

//! Tests for the v2 pipe format as the pipe runtime sees it: frontmatter
//! round trips, pipe id checks on manager operations, config changes,
//! status fields, structured scheduling and the prompt preamble.

use super::*;
use serde_json::json;

/// A pipe.md that sets every field added with the v2 format.
const V2_PIPE_MD: &str = r#"---
schedule: manual
enabled: true
agent: cloud-agent
cloud_agent:
  provider: cursor
  repository: octo/repo
  branch: main
  send_screenpipe_context: true
  context_lookback_hours: 12
  context_max_items: 40
effort: high
timeout: 600
trigger:
  events:
  - meeting_ended
  sources:
  - app: github
    path: octo/repo
    kind: issue_created
    filter: bug
  - app: voice
    filter:
      phrases:
      - remind me
history: true
run_in:
  mode: existing_chat
  chat_id: 0b9f3c4e-chat
artifacts:
- path: output/speaker-reconciliation.md
  title: Speaker reconciliation
  kind: markdown
- path: output/data.json
schedule_config:
  frequency: weeks
  interval: 2
  days_of_week:
  - 1
  - 4
  day_of_month: null
  at_hour: 9
  at_minute: 30
  timezone: Europe/Berlin
  starting: 2026-10-19T00:00:00+02:00
  ending: null
  max_occurrences: 10
event_read_only: true
title: Demo pipe
icon: "🧪"
---

Do the work.
"#;

fn manager(tmp: &tempfile::TempDir) -> PipeManager {
    let pipes_dir = tmp.path().join("pipes");
    std::fs::create_dir_all(&pipes_dir).unwrap();
    PipeManager::new(pipes_dir, HashMap::new(), None, 3035)
}

fn write_pipe(pm: &PipeManager, name: &str, content: &str) {
    let dir = pm.pipes_dir().join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("pipe.md"), content).unwrap();
}

fn updates(value: serde_json::Value) -> HashMap<String, serde_json::Value> {
    serde_json::from_value(value).unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

// -- frontmatter ---------------------------------------------------------------

#[test]
fn frontmatter_with_every_new_field_round_trips() {
    let (config, body) = parse_frontmatter(V2_PIPE_MD).unwrap();

    assert_eq!(config.effort, Some(PipeEffort::High));
    assert!(config.history);
    assert!(config.event_read_only);
    assert_eq!(
        config.run_in,
        Some(PipeRunIn::existing_chat("0b9f3c4e-chat"))
    );
    assert_eq!(config.artifacts.len(), 2);
    assert_eq!(
        config.artifacts[0].title.as_deref(),
        Some("Speaker reconciliation")
    );
    assert_eq!(config.artifacts[0].kind.as_deref(), Some("markdown"));
    let cloud = config.cloud_agent.as_ref().unwrap();
    assert_eq!(cloud.provider, "cursor");
    assert!(cloud.send_screenpipe_context);
    assert_eq!(cloud.context_max_items, Some(40));
    let trigger = config.trigger.as_ref().unwrap();
    assert_eq!(trigger.events, vec!["meeting_ended"]);
    assert_eq!(trigger.sources.len(), 2);
    assert_eq!(trigger.sources[0].kind.as_deref(), Some("issue_created"));
    assert_eq!(
        trigger.sources[1].filter,
        Some(json!({"phrases": ["remind me"]}))
    );
    let schedule = config.schedule_config.as_ref().unwrap();
    assert_eq!(schedule.frequency, ScheduleFrequency::Weeks);
    assert_eq!(schedule.days_of_week, vec![1, 4]);
    assert_eq!(
        schedule.starting.as_deref(),
        Some("2026-10-19T00:00:00+02:00")
    );
    assert!(schedule.compile().is_ok());
    // Typed fields never leak into the free-form extras; display extras stay.
    for typed in [
        "effort",
        "history",
        "run_in",
        "artifacts",
        "schedule_config",
        "cloud_agent",
        "event_read_only",
    ] {
        assert!(!config.config.contains_key(typed), "{typed} in extras");
    }
    assert_eq!(config.config.get("title"), Some(&json!("Demo pipe")));

    // Serialize and parse again: nothing is lost or changed.
    let serialized = serialize_pipe(&config, &body).unwrap();
    for key in [
        "cloud_agent:",
        "effort: high",
        "history: true",
        "run_in:",
        "artifacts:",
        "schedule_config:",
        "event_read_only: true",
        "sources:",
        "title: Speaker reconciliation",
    ] {
        assert!(serialized.contains(key), "missing {key}:\n{serialized}");
    }
    let (again, body_again) = parse_frontmatter(&serialized).unwrap();
    assert_eq!(body_again, body);
    assert_eq!(again.effort, config.effort);
    assert_eq!(again.history, config.history);
    assert_eq!(again.event_read_only, config.event_read_only);
    assert_eq!(again.run_in, config.run_in);
    assert_eq!(again.artifacts, config.artifacts);
    assert_eq!(again.cloud_agent, config.cloud_agent);
    assert_eq!(again.schedule_config, config.schedule_config);
    assert_eq!(
        again.trigger.as_ref().unwrap().sources,
        config.trigger.as_ref().unwrap().sources
    );
    assert_eq!(again.config, config.config);
}

#[test]
fn v1_frontmatter_still_loads_with_defaults() {
    let (config, _) = parse_frontmatter("---\nschedule: every 30m\n---\n\nBody").unwrap();
    assert!(config.effort.is_none() && config.run_in.is_none());
    assert!(config.artifacts.is_empty() && config.schedule_config.is_none());
    assert!(!config.history && !config.event_read_only);
    let serialized = serialize_pipe(&config, "Body").unwrap();
    for absent in [
        "effort",
        "history",
        "run_in",
        "artifacts",
        "schedule_config",
        "event_read_only",
    ] {
        assert!(
            !serialized.contains(absent),
            "{absent} written:\n{serialized}"
        );
    }
}

#[test]
fn hand_written_values_load_leniently() {
    let (config, _) = parse_frontmatter(
        "---\nhistory: yes\nevent_read_only: \"on\"\neffort: Medium\nartifacts:\n- output/x.md\n---\n\nB",
    )
    .unwrap();
    assert!(config.history && config.event_read_only);
    assert_eq!(config.effort, Some(PipeEffort::Medium));
    assert_eq!(config.artifacts[0].path, "output/x.md");
    let (nulls, _) =
        parse_frontmatter("---\nartifacts: null\ntrigger:\n  sources: null\n---\n\nB").unwrap();
    assert!(nulls.artifacts.is_empty());
    assert!(nulls.trigger.unwrap().sources.is_empty());
}

/// A pipe.md with a value this build cannot read in every v2 field.
const UNREADABLE_V2_PIPE_MD: &str = r#"---
schedule: every 1h
enabled: true
effort: max
history: maybe
run_in:
  chat_id: abc
cloud_agent:
  repository: octo/repo
artifacts: 5
schedule_config:
  frequency: daily
  at_hour: 9
event_read_only: sometimes
trigger:
  events:
  - meeting_ended
  sources:
  - path: octo/repo
title: Odd pipe
---

Do the work.
"#;

#[test]
fn unreadable_v2_values_are_set_aside_instead_of_failing_the_file() {
    let (config, body, notes) = parse_frontmatter_with_notes(UNREADABLE_V2_PIPE_MD).unwrap();
    assert_eq!(body, "Do the work.");
    assert_eq!(config.schedule, "every 1h");
    assert!(config.enabled);
    assert!(config.effort.is_none() && config.schedule_config.is_none());
    assert!(config.run_in.is_none() && config.cloud_agent.is_none());
    assert!(config.artifacts.is_empty());
    assert!(!config.history && !config.event_read_only);
    // The trigger keeps its events; only the unreadable sources are set aside.
    let trigger = config.trigger.as_ref().unwrap();
    assert_eq!(trigger.events, vec!["meeting_ended"]);
    assert!(trigger.sources.is_empty());
    assert_eq!(
        trigger.extra.get("sources"),
        Some(&json!([{"path": "octo/repo"}]))
    );
    // Each set-aside value is kept as written and named in a note.
    assert_eq!(config.config.get("effort"), Some(&json!("max")));
    assert_eq!(
        config.config.get("schedule_config"),
        Some(&json!({"frequency": "daily", "at_hour": 9}))
    );
    assert_eq!(config.config.get("history"), Some(&json!("maybe")));
    assert_eq!(config.config.get("title"), Some(&json!("Odd pipe")));
    for key in [
        "effort",
        "history",
        "run_in",
        "cloud_agent",
        "artifacts",
        "schedule_config",
        "event_read_only",
        "trigger.sources",
    ] {
        assert!(
            notes.iter().any(|n| n.starts_with(key)),
            "no note for {key}: {notes:?}"
        );
    }

    // A save writes the set-aside values back unchanged.
    let saved = serialize_pipe(&config, &body).unwrap();
    for kept in [
        "effort: max",
        "history: maybe",
        "frequency: daily",
        "artifacts: 5",
        "path: octo/repo",
    ] {
        assert!(saved.contains(kept), "{kept} lost:\n{saved}");
    }
    let (again, _) = parse_frontmatter(&saved).unwrap();
    assert_eq!(again.config, config.config);
    assert_eq!(again.trigger.unwrap().extra, trigger.extra);

    // A file that is broken outside the v2 fields still fails, and the
    // error names the real problem.
    assert!(parse_frontmatter("---\nschedule: [1, 2\n---\n\nB").is_err());
    let err = parse_frontmatter("---\ntimeout: soon\neffort: max\n---\n\nB")
        .unwrap_err()
        .to_string();
    assert!(err.contains("timeout"), "{err}");
}

#[tokio::test]
async fn a_pipe_with_unreadable_v2_values_loads_and_runs_on_its_schedule_string() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(
        &pm,
        "odd",
        "---\nschedule: every 1h\nenabled: true\neffort: max\nschedule_config:\n  frequency: daily\n---\n\nB",
    );
    pm.load_pipes().await.unwrap();
    let status = pm.get_pipe("odd").await.expect("the pipe is listed");
    assert!(status.config.schedule_config.is_none());
    // Interval schedules are measured against the real clock.
    let now = Utc::now();
    let half_an_hour_ago = now - chrono::Duration::minutes(30);
    assert!(pipe_should_run(
        &status.config,
        DateTime::<Utc>::UNIX_EPOCH,
        now
    ));
    assert!(!pipe_should_run(&status.config, half_an_hour_ago, now));
    assert_eq!(
        next_run_for(&status.config, Some(half_an_hour_ago), now),
        Some(half_an_hour_ago + chrono::Duration::hours(1))
    );
    assert_eq!(status.config.schedule_label(), "every 1h");
}

#[test]
fn builtin_pipes_still_parse() {
    for (name, content) in BUILTIN_PIPES {
        assert!(parse_frontmatter(content).is_ok(), "built-in {name}");
        assert!(is_bundled_builtin(name));
    }
    assert!(!is_bundled_builtin("my-own-pipe"));
}

// -- pipe ids --------------------------------------------------------------------

#[tokio::test]
async fn traversal_ids_are_rejected_by_every_operation() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    // A pipe-shaped folder next to the pipes directory that "../x" would hit.
    let outside = tmp.path().join("x");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        outside.join("pipe.md"),
        "---\nschedule: manual\n---\n\nOutside",
    )
    .unwrap();

    let is_invalid_id = |e: &anyhow::Error| {
        e.downcast_ref::<InvalidPipeId>().is_some()
            || matches!(
                e.downcast_ref::<PipeConfigUpdateError>(),
                Some(PipeConfigUpdateError::InvalidId(_))
            )
    };

    let err = pm
        .update_config(
            "../x",
            updates(json!({"raw_content": "---\nschedule: manual\n---\n\nPwned"})),
        )
        .await
        .unwrap_err();
    assert!(is_invalid_id(&err), "{err}");
    assert!(err.to_string().contains("invalid pipe identifier"), "{err}");

    for id in ["../x", "..\\x", "..", "CON", "a/b"] {
        assert!(
            is_invalid_id(&pm.enable_pipe(id, false).await.unwrap_err()),
            "{id}"
        );
        assert!(
            is_invalid_id(&pm.delete_pipe(id).await.unwrap_err()),
            "{id}"
        );
        assert!(
            is_invalid_id(&pm.clear_pipe_history(id).await.unwrap_err()),
            "{id}"
        );
        assert!(is_invalid_id(&pm.stop_pipe(id).await.unwrap_err()), "{id}");
        assert!(is_invalid_id(&pm.run_pipe(id).await.unwrap_err()), "{id}");
        assert!(
            is_invalid_id(&pm.start_pipe_background(id).await.unwrap_err()),
            "{id}"
        );
        assert!(
            is_invalid_id(
                &pm.install_pipe_from_store("---\nschedule: manual\n---\n\nB", id, 1)
                    .await
                    .unwrap_err()
            ),
            "{id}"
        );
    }

    // The folder outside the pipes directory was not touched.
    let content = std::fs::read_to_string(outside.join("pipe.md")).unwrap();
    assert!(content.contains("Outside"));
}

#[tokio::test]
async fn install_from_url_rejects_unsafe_derived_names() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    // A URL whose last segment is empty would have written pipes/pipe.md.
    let err = pm
        .install_pipe("https://example.invalid/")
        .await
        .unwrap_err();
    assert!(err.downcast_ref::<InvalidPipeId>().is_some(), "{err}");
    assert!(!pm.pipes_dir().join("pipe.md").exists());
    assert_eq!(url_to_pipe_name("https://h/p/daily.md?raw=1#top"), "daily");
}

// -- config changes --------------------------------------------------------------

#[tokio::test]
async fn config_update_rejects_unknown_fields_and_envelopes() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(
        &pm,
        "demo",
        "---\nschedule: every 1h\nenabled: true\n---\n\nBody",
    );
    pm.load_pipes().await.unwrap();
    let before = std::fs::read_to_string(pm.pipes_dir().join("demo/pipe.md")).unwrap();

    let err = pm
        .update_config(
            "demo",
            updates(json!({"schedule": "every 2h", "colour": "red"})),
        )
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<PipeConfigUpdateError>(),
        Some(&PipeConfigUpdateError::UnknownField("colour".into()))
    );
    let err = pm
        .update_config("demo", updates(json!({"config": {"schedule": "every 2h"}})))
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<PipeConfigUpdateError>(),
        Some(&PipeConfigUpdateError::Envelope)
    );
    let err = pm
        .update_config("missing", updates(json!({"enabled": false})))
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<PipeConfigUpdateError>(),
        Some(&PipeConfigUpdateError::NotFound("missing".into()))
    );
    // Refused changes leave the file alone.
    let after = std::fs::read_to_string(pm.pipes_dir().join("demo/pipe.md")).unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn config_update_writes_typed_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(
        &pm,
        "demo",
        "---\nschedule: every 1h\nenabled: true\n---\n\nBody",
    );
    pm.load_pipes().await.unwrap();

    pm.update_config(
        "demo",
        updates(json!({
            "schedule_config": {"frequency": "days", "interval": 1, "at_hour": 9, "at_minute": 0,
                                "timezone": "UTC"},
            "effort": "low",
            "history": true,
            "enabled": false,
            "run_in": {"mode": "existing_chat", "chat_id": "chat-1"},
        })),
    )
    .await
    .unwrap();

    let written = std::fs::read_to_string(pm.pipes_dir().join("demo/pipe.md")).unwrap();
    let (config, body) = parse_frontmatter(&written).unwrap();
    assert_eq!(body, "Body");
    assert_eq!(config.effort, Some(PipeEffort::Low));
    assert!(config.history);
    assert!(!config.enabled);
    assert_eq!(
        config.schedule_config.as_ref().map(|s| s.frequency),
        Some(ScheduleFrequency::Days)
    );
    // The enabled choice is also this device's override.
    assert_eq!(
        load_local_overrides(pm.pipes_dir()).get("demo"),
        Some(&false)
    );
    // In-memory state follows the file.
    let status = pm.get_pipe("demo").await.unwrap();
    assert_eq!(status.config.effort, Some(PipeEffort::Low));
    assert_eq!(status.config.schedule_label(), "Every day at 09:00 (UTC)");
}

#[tokio::test]
async fn raw_content_is_written_verbatim_and_validated() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(&pm, "demo", "---\nschedule: every 1h\n---\n\nBody");
    pm.load_pipes().await.unwrap();

    let raw = "---\n# keep this comment\nschedule: every 2h\neffort: medium\n---\n\nNew body\n";
    pm.update_config("demo", updates(json!({"raw_content": raw})))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(pm.pipes_dir().join("demo/pipe.md")).unwrap(),
        raw
    );

    let bad =
        "---\nschedule: every 2h\nrun_in:\n  mode: existing_chat\n  chat_id: ../../x\n---\n\nB";
    let err = pm
        .update_config("demo", updates(json!({"raw_content": bad})))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<PipeConfigUpdateError>(),
            Some(PipeConfigUpdateError::Invalid { field, .. }) if field == "run_in"
        ),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(pm.pipes_dir().join("demo/pipe.md")).unwrap(),
        raw
    );
}

#[tokio::test]
async fn enabling_a_finished_structured_schedule_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(
        &pm,
        "ended",
        "---\nschedule: manual\nenabled: false\nschedule_config:\n  frequency: days\n  timezone: UTC\n  ending: 2020-01-01T00:00:00Z\n---\n\nB",
    );
    pm.load_pipes().await.unwrap();
    let err = pm.enable_pipe("ended", true).await.unwrap_err();
    assert!(err.to_string().contains("already ended"), "{err}");
    // Disabling is always allowed.
    pm.enable_pipe("ended", false).await.unwrap();
}

#[tokio::test]
async fn enabling_a_finished_structured_schedule_through_config_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(
        &pm,
        "ended",
        "---\nschedule: manual\nenabled: false\nschedule_config:\n  frequency: days\n  timezone: UTC\n  ending: 2020-01-01T00:00:00Z\n---\n\nB",
    );
    pm.load_pipes().await.unwrap();
    let before = std::fs::read_to_string(pm.pipes_dir().join("ended/pipe.md")).unwrap();
    let err = pm
        .update_config("ended", updates(json!({"enabled": true})))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err.downcast_ref::<PipeConfigUpdateError>(),
            Some(PipeConfigUpdateError::Invalid { field, .. }) if field == "enabled"
        ),
        "{err}"
    );
    assert!(err.to_string().contains("already ended"), "{err}");
    assert_eq!(
        std::fs::read_to_string(pm.pipes_dir().join("ended/pipe.md")).unwrap(),
        before
    );
    assert_ne!(
        load_local_overrides(pm.pipes_dir()).get("ended"),
        Some(&true)
    );
    // Enabling together with a new end date is fine.
    pm.update_config(
        "ended",
        updates(json!({
            "enabled": true,
            "schedule_config": {"frequency": "days", "timezone": "UTC", "ending": "2999-01-01"}
        })),
    )
    .await
    .unwrap();
    assert_eq!(
        load_local_overrides(pm.pipes_dir()).get("ended"),
        Some(&true)
    );
}

#[tokio::test]
async fn a_schedule_string_sent_alone_replaces_the_structured_schedule() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    // Written elsewhere: a structured schedule and no `schedule` string, so
    // the settings UI shows the default "manual".
    let structured = "---\nenabled: true\nschedule_config:\n  frequency: minutes\n  interval: 5\n  timezone: UTC\n---\n\nB";
    write_pipe(&pm, "structured", structured);
    write_pipe(&pm, "both", structured);
    pm.load_pipes().await.unwrap();
    let now = Utc::now();
    let status = pm.get_pipe("structured").await.unwrap();
    assert_eq!(status.config.schedule, "manual");
    assert!(status.next_run.is_some());

    // Choosing "manual" in the UI now stops the timed runs.
    pm.update_config("structured", updates(json!({"schedule": "manual"})))
        .await
        .unwrap();
    let status = pm.get_pipe("structured").await.unwrap();
    assert!(status.config.schedule_config.is_none());
    assert!(status.next_run.is_none(), "{:?}", status.next_run);
    assert!(!pipe_should_run(
        &status.config,
        DateTime::<Utc>::UNIX_EPOCH,
        now
    ));
    let written = std::fs::read_to_string(pm.pipes_dir().join("structured/pipe.md")).unwrap();
    assert!(!written.contains("schedule_config"), "{written}");

    // Another string choice takes over the same way.
    pm.update_config("structured", updates(json!({"schedule": "every 2h"})))
        .await
        .unwrap();
    let status = pm.get_pipe("structured").await.unwrap();
    assert_eq!(status.config.schedule_label(), "every 2h");

    // Sent together, the structured schedule stays.
    pm.update_config(
        "both",
        updates(json!({
            "schedule": "every 2h",
            "schedule_config": {"frequency": "days", "at_hour": 9, "timezone": "UTC"}
        })),
    )
    .await
    .unwrap();
    let both = pm.get_pipe("both").await.unwrap();
    assert_eq!(
        both.config.schedule_config.map(|s| s.frequency),
        Some(ScheduleFrequency::Days)
    );
}

#[tokio::test]
async fn values_already_on_disk_do_not_block_unrelated_saves() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    // Loads fine, but several values would not pass the API's checks.
    let file = "---\nschedule: every 1h\nenabled: true\neffort: max\nrun_in:\n  mode: new_chat\nartifacts:\n- path: output/x.csv\n  kind: csv\ntrigger:\n  events:\n  - meeting_ended\n  sources:\n  - app: myspace\ncolour: red\n---\n\nBody";
    write_pipe(&pm, "lenient", file);
    pm.load_pipes().await.unwrap();
    let view =
        || async { serde_json::to_value(pm.get_pipe("lenient").await.unwrap().config).unwrap() };

    // The prompt editor saves the whole file with only the body changed.
    let edited = file.replace("Body", "New body");
    pm.update_config("lenient", updates(json!({"raw_content": edited})))
        .await
        .unwrap();
    // The notification toggle and other single-field saves.
    pm.update_config("lenient", updates(json!({"history": true})))
        .await
        .unwrap();
    // The trigger editor posts the whole trigger back with one change.
    let mut trigger = view().await["trigger"].clone();
    trigger["events"] = json!(["meeting_ended", "meeting_started"]);
    pm.update_config("lenient", updates(json!({"trigger": trigger})))
        .await
        .unwrap();
    let status = pm.get_pipe("lenient").await.unwrap();
    let trigger = status.config.trigger.as_ref().unwrap();
    assert_eq!(trigger.events, vec!["meeting_ended", "meeting_started"]);
    assert_eq!(trigger.sources[0].app, "myspace");
    assert_eq!(status.prompt_body, "New body");

    // A full-object save echoes every field, free-form extras included.
    let mut whole: HashMap<String, serde_json::Value> =
        serde_json::from_value(view().await).unwrap();
    whole.insert("timeout".into(), json!(900));
    pm.update_config("lenient", whole).await.unwrap();
    let status = pm.get_pipe("lenient").await.unwrap();
    assert_eq!(status.config.timeout, Some(900));
    assert_eq!(status.config.config.get("effort"), Some(&json!("max")));
    assert_eq!(status.config.config.get("colour"), Some(&json!("red")));

    // New bad values are still refused.
    for (change, field) in [
        (
            json!({"trigger": {"events": ["meeting_ended"], "sources": [{"app": "myspace"}, {"app": "fax"}]}}),
            "trigger",
        ),
        (
            json!({"artifacts": [{"path": "output/x.csv", "kind": "csv"}, {"path": "../y.md"}]}),
            "artifacts",
        ),
        (
            json!({"raw_content": file.replace("effort: max", "effort: extreme")}),
            "effort",
        ),
    ] {
        match pm.update_config("lenient", updates(change.clone())).await {
            Err(e) => assert!(
                matches!(
                    e.downcast_ref::<PipeConfigUpdateError>(),
                    Some(PipeConfigUpdateError::Invalid { field: f, .. }) if f == field
                ),
                "{change}: {e}"
            ),
            Ok(()) => panic!("{change} should be refused"),
        }
    }
    // An extra the file does not have is still an unknown field.
    let err = pm
        .update_config("lenient", updates(json!({"colour": "blue"})))
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<PipeConfigUpdateError>(),
        Some(&PipeConfigUpdateError::UnknownField("colour".into()))
    );
}

#[tokio::test]
async fn pipes_in_folders_named_before_the_id_rules_can_still_be_managed() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    let mut names = vec![".dotted"];
    if cfg!(not(windows)) {
        names.extend(["aux", "trailing.", "daily.md?raw=1"]);
    }
    for name in &names {
        write_pipe(
            &pm,
            name,
            "---\nschedule: every 1h\nenabled: true\n---\n\nB",
        );
    }
    pm.load_pipes().await.unwrap();
    for name in names {
        assert!(validate_pipe_id(name).is_err(), "{name:?}");
        assert!(pm.get_pipe(name).await.is_some(), "{name:?} loads");
        pm.enable_pipe(name, false).await.unwrap();
        pm.update_config(name, updates(json!({"prompt_body": "Edited"})))
            .await
            .unwrap();
        pm.stop_pipe(name).await.unwrap();
        pm.clear_pipe_history(name).await.unwrap();
        pm.delete_pipe(name).await.unwrap();
        assert!(!pm.pipes_dir().join(name).exists(), "{name:?} deleted");
    }
    // New pipes still get the full rules.
    assert!(pm
        .install_pipe_from_store("---\nschedule: manual\n---\n\nB", ".dotted", 1)
        .await
        .is_err());
}

// -- status ------------------------------------------------------------------------

#[tokio::test]
async fn status_reports_builtin_saved_context_and_next_run() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(
        &pm,
        "day-recap",
        "---\nschedule: manual\n---\n\nBuilt-in copy",
    );
    write_pipe(
        &pm,
        "hourly",
        "---\nschedule: every 1h\nenabled: true\nhistory: true\n---\n\nB",
    );
    std::fs::write(
        pm.pipes_dir()
            .join("hourly")
            .join(".screenpipe-acp-sessions.json"),
        r#"{"claude": "session-1"}"#,
    )
    .unwrap();
    pm.load_pipes().await.unwrap();

    let statuses = pm.list_pipes().await;
    let recap = statuses
        .iter()
        .find(|s| s.config.name == "day-recap")
        .unwrap();
    let hourly = statuses.iter().find(|s| s.config.name == "hourly").unwrap();
    assert!(recap.is_bundled_builtin && !hourly.is_bundled_builtin);
    assert!(hourly.has_saved_context && !recap.has_saved_context);
    assert!(recap.next_run.is_none(), "manual pipes have no next run");
    assert!(
        hourly.next_run.is_some(),
        "a never-run interval pipe is due"
    );
    assert!(hourly.execution_count.is_none());

    let one = pm.get_pipe("hourly").await.unwrap();
    assert!(one.has_saved_context && one.next_run.is_some());
    let json = serde_json::to_value(&one).unwrap();
    for key in ["has_saved_context", "is_bundled_builtin", "next_run"] {
        assert!(json.get(key).is_some(), "{key} missing from {json}");
    }
}

#[test]
fn next_run_per_schedule_kind() {
    let now = utc("2026-10-20T10:00:00Z");
    let cfg = |yaml: &str| {
        let (mut c, _) = parse_frontmatter(&format!("---\n{yaml}\n---\n\nB")).unwrap();
        c.name = "p".into();
        c
    };
    let interval = cfg("schedule: every 2h");
    assert_eq!(next_run_for(&interval, None, now), Some(now));
    assert_eq!(
        next_run_for(&interval, Some(utc("2026-10-20T09:00:00Z")), now),
        Some(utc("2026-10-20T11:00:00Z"))
    );
    assert_eq!(
        next_run_for(&interval, Some(utc("2026-10-20T01:00:00Z")), now),
        Some(now),
        "an overdue run is due now"
    );
    assert_eq!(next_run_for(&cfg("schedule: manual"), None, now), None);
    assert_eq!(
        next_run_for(&cfg("schedule: every 2h\nenabled: false"), None, now),
        None
    );
    let once = cfg("schedule: at 2026-10-20T12:00:00Z");
    assert_eq!(
        next_run_for(&once, None, now),
        Some(utc("2026-10-20T12:00:00Z"))
    );
    assert_eq!(
        next_run_for(&once, Some(utc("2026-10-20T12:00:01Z")), now),
        None
    );
    let structured = cfg(
        "schedule: every 5m\nschedule_config:\n  frequency: days\n  at_hour: 9\n  timezone: UTC",
    );
    assert_eq!(
        next_run_for(&structured, Some(utc("2026-10-20T09:00:00Z")), now),
        Some(utc("2026-10-21T09:00:00Z")),
        "schedule_config wins over the schedule string"
    );
    let ended =
        cfg("schedule_config:\n  frequency: days\n  timezone: UTC\n  ending: 2026-10-01T00:00:00Z");
    assert_eq!(next_run_for(&ended, None, now), None);
}

// -- structured scheduling ---------------------------------------------------------

#[test]
fn structured_schedule_drives_should_run() {
    let (mut config, _) = parse_frontmatter(
        "---\nschedule: every 1m\nschedule_config:\n  frequency: days\n  at_hour: 9\n  timezone: UTC\n---\n\nB",
    )
    .unwrap();
    config.name = "p".into();
    let at = |s: &str| utc(s);
    // Ran yesterday at 09:00; at 08:59 nothing is due even though the
    // schedule string alone would fire every minute.
    assert!(!pipe_should_run(
        &config,
        at("2026-10-19T09:00:00Z"),
        at("2026-10-20T08:59:00Z")
    ));
    assert!(pipe_should_run(
        &config,
        at("2026-10-19T09:00:00Z"),
        at("2026-10-20T09:00:30Z")
    ));
    // Fresh install after the slot: wait for tomorrow.
    assert!(!pipe_should_run(
        &config,
        DateTime::<Utc>::UNIX_EPOCH,
        at("2026-10-20T09:30:00Z")
    ));
    // Fresh install inside the grace window: run.
    assert!(pipe_should_run(
        &config,
        DateTime::<Utc>::UNIX_EPOCH,
        at("2026-10-20T09:03:00Z")
    ));
    // App was asleep at 09:00 and wakes at 15:00: catch up.
    assert!(pipe_should_run(
        &config,
        at("2026-10-19T09:00:00Z"),
        at("2026-10-20T15:00:00Z")
    ));

    // An invalid structured schedule is ignored: the schedule string (what
    // the settings UI shows) drives the pipe.
    config.schedule_config.as_mut().unwrap().at_hour = Some(99);
    assert!(pipe_should_run(
        &config,
        at("2026-10-20T08:58:00Z"),
        at("2026-10-20T08:59:30Z")
    ));
    config.schedule = "manual".into();
    assert!(!pipe_should_run(
        &config,
        DateTime::<Utc>::UNIX_EPOCH,
        at("2026-10-20T09:00:00Z")
    ));
}

#[test]
fn structured_schedule_finishes_after_its_last_run() {
    let (config, _) = parse_frontmatter(
        "---\nschedule_config:\n  frequency: days\n  at_hour: 9\n  timezone: UTC\n  starting: 2026-10-19T00:00:00Z\n  max_occurrences: 2\n---\n\nB",
    )
    .unwrap();
    // Before the second run, the schedule is still live.
    assert!(!structured_schedule_finished(
        &config,
        utc("2026-10-19T09:00:00Z"),
        utc("2026-10-20T08:00:00Z")
    ));
    // The second (last) run is due, so it is not finished yet...
    assert!(!structured_schedule_finished(
        &config,
        utc("2026-10-19T09:00:00Z"),
        utc("2026-10-20T09:00:10Z")
    ));
    // ...and once it ran, it is.
    assert!(structured_schedule_finished(
        &config,
        utc("2026-10-20T09:00:10Z"),
        utc("2026-10-20T09:00:40Z")
    ));
    // A plain schedule string never "finishes".
    let (plain, _) = parse_frontmatter("---\nschedule: every 1h\n---\n\nB").unwrap();
    assert!(!structured_schedule_finished(
        &plain,
        DateTime::<Utc>::UNIX_EPOCH,
        utc("2026-10-20T09:00:00Z")
    ));
}

#[tokio::test]
async fn finishing_a_schedule_disables_the_pipe_locally() {
    let tmp = tempfile::tempdir().unwrap();
    let pm = manager(&tmp);
    write_pipe(&pm, "p", "---\nschedule: manual\nenabled: true\n---\n\nB");
    pm.load_pipes().await.unwrap();
    finish_structured_schedule(&pm.pipes, pm.pipes_dir(), "p").await;
    assert_eq!(load_local_overrides(pm.pipes_dir()).get("p"), Some(&false));
    assert!(!pm.get_pipe("p").await.unwrap().config.enabled);
}

#[test]
fn lookback_follows_the_structured_interval() {
    let (mut config, _) = parse_frontmatter(
        "---\nschedule: every 6h\nschedule_config:\n  frequency: minutes\n  interval: 20\n---\n\nB",
    )
    .unwrap();
    config.name = "p".into();
    let prompt = render_prompt_with_port(&config, "", 3035, None, None);
    let range = prompt.lines().next().unwrap();
    let parts: Vec<&str> = range
        .trim_start_matches("Time range: ")
        .split(" to ")
        .collect();
    let start = utc(parts[0]);
    let end = utc(parts[1]);
    assert_eq!((end - start).num_minutes(), 20, "{range}");
}

// -- preamble ------------------------------------------------------------------------

#[test]
fn preamble_carries_the_four_directives() {
    let mut config = PipeConfig {
        name: "daily digest".into(),
        ..Default::default()
    };
    // The ratings directive waits for the feedback route.
    let guidance = render_pipe_guidance(&config, 3035, true);
    assert!(
        guidance.contains("GET http://localhost:3035/feedback?producer=pipe:daily%20digest"),
        "{guidance}"
    );
    assert!(!render_pipe_guidance(&config, 3035, false).contains("/feedback"));
    let sys = render_pipe_system_prompt(&config, "BODY", 3035, None, None, None);
    assert_eq!(
        sys.contains("/feedback?producer="),
        FEEDBACK_ROUTE_AVAILABLE,
        "{sys}"
    );
    assert!(sys.contains("\"actions\""), "{sys}");
    assert!(sys.contains("user_profile"), "{sys}");
    assert!(
        !sys.contains("Sub-agents"),
        "sub-agent guidance only when enabled"
    );
    // The guidance comes before the pipe's own instructions.
    assert!(sys.find("How to work with the user").unwrap() < sys.find("BODY").unwrap());

    config.subagent = true;
    let sys = render_pipe_system_prompt(&config, "BODY", 3035, None, None, None);
    assert!(sys.contains("Sub-agents are switched on"), "{sys}");
    assert!(sys.contains("two or more"), "{sys}");
}

#[test]
fn query_escape_keeps_safe_characters() {
    assert_eq!(query_escape("pipe:abc-1_2.3~"), "pipe:abc-1_2.3~");
    assert_eq!(query_escape("a b&c=é"), "a%20b%26c%3D%C3%A9");
}

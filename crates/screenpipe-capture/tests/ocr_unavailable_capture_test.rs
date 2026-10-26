// Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
// https://github.com/normiecore/Freenpipe

//! Capture with OCR reported missing (the `FREENPIPE_OCR_FAULT` hook).
//!
//! The hook is a process-wide switch, so this test lives in its own test
//! binary: no other test can run while it is on, and a guard turns it off
//! again even when an assertion fails.

use std::sync::Arc;

use chrono::Utc;
use image::{DynamicImage, RgbImage};
use screenpipe_a11y::tree::{AccessibilityTreeNode, TreeSnapshot, TruncationReason};
use screenpipe_capture::{paired_capture, CaptureContext};
use screenpipe_db::DatabaseManager;
use screenpipe_screen::ocr_availability::simulate_ocr_unavailable;
use screenpipe_screen::snapshot_writer::SnapshotWriter;
use tempfile::TempDir;

/// Turns the simulated "no OCR installed" state on, and off again on drop.
struct OcrUnavailable;

impl OcrUnavailable {
    fn on() -> Self {
        simulate_ocr_unavailable(true);
        Self
    }
}

impl Drop for OcrUnavailable {
    fn drop(&mut self) {
        simulate_ocr_unavailable(false);
    }
}

/// A frame whose accessibility text is thin still runs the OCR step, gets
/// nothing back, and is stored with its accessibility text; a frame with no
/// accessibility text at all is still stored.
#[tokio::test]
async fn capture_keeps_accessibility_text_when_ocr_is_unavailable() {
    let _ocr_off = OcrUnavailable::on();
    let tmp = TempDir::new().unwrap();
    let snapshot_writer = SnapshotWriter::new(tmp.path(), 80, 1920);
    let db = DatabaseManager::new("sqlite::memory:", Default::default())
        .await
        .unwrap();
    let now = Utc::now();

    let ctx = CaptureContext {
        db: &db,
        snapshot_writer: &snapshot_writer,
        image: Arc::new(DynamicImage::ImageRgb8(RgbImage::new(100, 100))),
        captured_at: now,
        monitor_id: 0,
        device_name: "test_monitor",
        app_name: Some("Google Chrome"),
        window_name: Some("Quarterly plan - Google Docs"),
        browser_url: Some("https://docs.google.com/document/d/abc/edit"),
        document_path: None,
        focused: true,
        capture_trigger: "click",
        use_pii_removal: false,
        languages: vec![],
        elements_ref_frame_id: None,
        screenshot_disabled: false,
        pixels_withheld: false,
    };
    let snap = TreeSnapshot {
        app_name: "Google Chrome".to_string(),
        window_name: "Quarterly plan - Google Docs".to_string(),
        text_content: "File Edit View Insert Format".to_string(),
        nodes: vec![AccessibilityTreeNode {
            role: "AXStaticText".to_string(),
            text: "File Edit View Insert Format".to_string(),
            depth: 0,
            bounds: None,
            ..Default::default()
        }],
        browser_url: Some("https://docs.google.com/document/d/abc/edit".to_string()),
        document_path: None,
        timestamp: now,
        node_count: 1,
        walk_duration: std::time::Duration::from_millis(5),
        content_hash: 111,
        simhash: 222,
        truncated: false,
        truncation_reason: TruncationReason::None,
        max_depth_reached: 0,
    };
    let result = paired_capture(&ctx, Some(&snap)).await.unwrap();
    assert!(result.frame_id > 0);
    assert_eq!(result.text_source.as_deref(), Some("accessibility"));
    assert_eq!(
        result.accessibility_text.as_deref(),
        Some("File Edit View Insert Format")
    );

    let bare = CaptureContext {
        app_name: Some("SomeGame"),
        window_name: Some("Level 1"),
        browser_url: None,
        ..ctx
    };
    let result = paired_capture(&bare, None).await.unwrap();
    assert!(result.frame_id > 0);
    assert!(result.text_source.is_none());
}

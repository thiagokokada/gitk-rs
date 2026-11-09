//! Opt-in release probe of the application's real views on GPUI's test platform.
use crate::ui::tests::{history_fixture, open_with_batch, until};
use gpui_kit::{
    Modifiers, Pixels, Point, ScrollDelta, ScrollEvent, TestAppContext, TouchPhase,
    VisualTestContext, point, px, test::TestWindowExt,
};
use std::time::{Duration, Instant};

fn draw(cx: &mut VisualTestContext) {
    // Do not use refresh()/render_frame(): forcing a refresh bypasses retention.
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext, mut ready: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        cx.background_executor
            .advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        draw(cx);
        if ready(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "profile worker did not settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn scroll(cx: &mut VisualTestContext, position: Point<Pixels>, delta: f32) -> Duration {
    let started = Instant::now();
    cx.simulate_event(ScrollEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.), px(delta))),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    draw(cx);
    started.elapsed()
}

fn report(scenario: &str, mut samples: Vec<Duration>) {
    samples.sort_unstable();
    let median = samples[samples.len() / 2].as_micros();
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1].as_micros();
    eprintln!(
        "ui_profile backend={} scenario={scenario} samples={} median_us={median} p95_us={p95}",
        if gpui_kit::GPUI_FAST {
            "fast"
        } else {
            "upstream"
        },
        samples.len(),
    );
}

#[gpui_kit::test]
#[ignore = "opt-in release UI preparation probe; no GPU or display timing"]
fn profile_ui_scrolling(cx: &mut TestAppContext) {
    cx.disable_accessibility();
    let fixture = history_fixture(5000);
    let source: String = (0..2000)
        .map(|i| format!("pub fn value_{i}() -> usize {{ {i} }}\n"))
        .collect();
    std::fs::write(fixture.root().join("profile.rs"), source).unwrap();
    let (app, cx) = open_with_batch(&fixture, None, cx);
    let pane = cx.update(|_, cx| app.view.read(cx).diff.clone());
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.loaded == 5000)
                && !app.view.read(cx).local_rows.is_empty()
        })
    });
    cx.simulate_keystrokes("home");
    settle(cx, |cx| {
        cx.update(|_, cx| {
            pane.read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.count > 2000 && !s.highlighting && s.error.is_none())
        })
    });
    eprintln!(
        "ui_profile fixture_head={} commits=5000 diff_source_lines=2000 window=1280x850 scale=2 graph_cols=256 accessibility=off",
        fixture.git(&["rev-parse", "HEAD"])
    );

    for scenario in ["history", "diff"] {
        #[cfg(feature = "gpui-fast")]
        cx.update(|window, _| window.reset_layout_stats());
        let position = cx.update(|window, _| {
            window
                .find(if scenario == "history" {
                    "history-scrollbar-viewport"
                } else {
                    "diff-content"
                })
                .bounds()
                .center()
        });
        let mut samples = Vec::with_capacity(240);
        for frame in 0..272 {
            // Oscillate within the fixture; discard the first complete cycle.
            let delta = if frame % 32 < 16 { -40. } else { 40. };
            let elapsed = scroll(cx, position, delta);
            if frame == 0 {
                let offset = cx.update(|_, cx| {
                    if scenario == "history" {
                        app.view.read(cx).scroll.0.borrow().base_handle.offset().y
                    } else {
                        pane.read(cx).scroll.0.borrow().base_handle.offset().y
                    }
                });
                assert!(offset < px(0.), "{scenario} wheel did not scroll");
            }
            if frame >= 32 {
                samples.push(elapsed);
            }
            // Let real worker threads supply the next page outside the sample.
            // Draw without forced refresh so the next sample can reuse views.
            settle(cx, |cx| {
                cx.update(|_, cx| {
                    let view = app.view.read(cx);
                    let history_ready = view.snapshot.as_ref().is_some_and(|s| {
                        view.viewport.start.saturating_sub(view.local_rows.len()) >= s.start
                            && view
                                .viewport
                                .end
                                .saturating_sub(view.local_rows.len())
                                .min(s.visible)
                                <= s.start + s.rows.len()
                    });
                    let diff = pane.read(cx);
                    history_ready
                        && diff.snapshot.as_ref().is_some_and(|s| {
                            !s.highlighting
                                && s.error.is_none()
                                && s.start <= diff.viewport.start
                                && s.start + s.lines.len() >= diff.viewport.end.min(s.count)
                        })
                })
            });
        }
        report(scenario, samples);
        #[cfg(feature = "gpui-fast")]
        cx.update(|window, _| {
            let stats = window.layout_stats();
            eprintln!(
                "ui_profile scenario={scenario} all_draws={} views_built={} views_reused={} layer_composited={} layer_repainted={}",
                stats.frames, stats.views_built, stats.views_reused,
                stats.layer_frames_composited, stats.layer_frames_repainted,
            );
        });
    }
}

#[gpui_kit::test]
#[ignore = "opt-in release virtual versus eager history comparison"]
fn profile_commit_list(cx: &mut TestAppContext) {
    const WARMUP: usize = 32;
    const SAMPLES: usize = 96;
    let count: usize = std::env::var("GITK_PROFILE_ROWS")
        .unwrap_or_else(|_| "5000".into())
        .parse()
        .expect("GITK_PROFILE_ROWS must be an integer");
    assert!((100..=10000).contains(&count), "use 100..10000 rows");
    let strategy = std::env::var("GITK_PROFILE_LIST").unwrap_or_else(|_| "virtual".into());
    assert!(matches!(strategy.as_str(), "virtual" | "eager"));
    cx.disable_accessibility();
    let fixture = history_fixture(count);
    let (app, cx) = open_with_batch(&fixture, None, cx);
    until(cx, |cx| {
        cx.update(|_, cx| {
            app.view
                .read(cx)
                .snapshot
                .as_ref()
                .is_some_and(|s| s.eof && s.loaded == count)
        })
    });
    // Give both strategies the same complete display snapshot, so this compares
    // UI construction rather than missing-page requests or worker latency.
    // Worker snapshots are capped at PAGE_ROWS. Collect successive pages while
    // the test clock is stopped, then install their shared entries in one
    // test-only display snapshot. Do not relax production worker bounds.
    let (handle, mut request, mut snapshot) = cx.update(|_, cx| {
        let view = app.view.read(cx);
        (
            view.history.clone(),
            view.request.clone(),
            view.snapshot.clone().unwrap(),
        )
    });
    let mut rows = Vec::with_capacity(count);
    for start in (0..count).step_by(gitk_rs::history::PAGE_ROWS) {
        request.id += 1;
        request.range = start..start + gitk_rs::history::PAGE_ROWS;
        handle.request(request.clone());
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(page) = handle.take_snapshot()
                && page.request_id == request.id
            {
                assert_eq!(page.start, start);
                assert_eq!(
                    page.rows.len(),
                    (count - start).min(gitk_rs::history::PAGE_ROWS)
                );
                rows.extend(page.rows.iter().cloned());
                snapshot = page;
                break;
            }
            assert!(
                Instant::now() < deadline,
                "profile history page did not arrive"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    snapshot.start = 0;
    snapshot.rows = rows;
    app.view.update(cx, |view, _| {
        view.request = request;
        view.snapshot = Some(snapshot);
    });
    let started = Instant::now();
    app.view.update(cx, |view, cx| {
        view.profile_eager_history = strategy == "eager";
        cx.notify();
    });
    draw(cx);
    let first_draw = started.elapsed();
    eprintln!(
        "ui_profile list={strategy} rows={count} fixture_head={} first_draw_us={} window=1280x850 scale=2 graph_cols=256 accessibility=off",
        fixture.git(&["rev-parse", "HEAD"]),
        first_draw.as_micros(),
    );
    let position =
        cx.update(|window, _| window.find("history-scrollbar-viewport").bounds().center());
    let mut samples = Vec::with_capacity(SAMPLES);
    for frame in 0..WARMUP + SAMPLES {
        let delta = if frame % 32 < 16 { -40. } else { 40. };
        let elapsed = scroll(cx, position, delta);
        if frame == 0 {
            assert!(
                cx.update(|_, cx| app.view.read(cx).scroll.0.borrow().base_handle.offset().y)
                    < px(0.),
                "{strategy} list did not scroll"
            );
        }
        if frame >= WARMUP {
            samples.push(elapsed);
        }
        settle(cx, |_| true);
    }
    report(&format!("commit-list-{strategy}-{count}"), samples);
}

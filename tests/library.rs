//! Opt-in real-engine test, intentionally importing only Bug's Rust API.
use bug::{
    Browser, BrowserEvent, BrowserOptions, EvaluationResult, Frame, KeyInput, SessionSnapshot,
    ViewMetadata, Viewport,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn until(browser: &mut Browser, mut predicate: impl FnMut(&BrowserEvent) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        browser.poll();
        let events: Vec<_> = browser.drain_events().collect();
        let mut done = false;
        for event in events {
            assert!(!matches!(event, BrowserEvent::Error { .. }), "{event:?}");
            done |= predicate(&event);
        }
        if done {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for library event"
        );
        std::thread::sleep(Duration::from_millis(8));
    }
}
fn frame(browser: &mut Browser, view: u64, width: u32, height: u32) -> Frame {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut frame = Frame::default();
    loop {
        browser.poll();
        for event in browser.drain_events() {
            assert!(!matches!(event, BrowserEvent::Error { .. }), "{event:?}");
        }
        if browser.read_frame(view, &mut frame).unwrap()
            && frame.width == width
            && frame.height == height
            && frame.pixels.contains(&0xeef3eb)
            && frame.pixels.contains(&0x577c50)
        {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for painted frame"
        );
        std::thread::sleep(Duration::from_millis(8));
    }
}

#[test]
#[ignore = "requires the built native engine; run with DISPLAY and WAYLAND_DISPLAY unset"]
fn independent_rust_api_without_a_display_or_json() {
    assert!(std::env::var_os("DISPLAY").is_none());
    assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let profile = std::env::temp_dir().join(format!("bug-api-{}", std::process::id()));
    let options = BrowserOptions {
        profile: profile.clone(),
        ..Default::default()
    };
    let mut browser = Browser::new(options.clone()).unwrap();
    assert!(
        Browser::new(options).is_err(),
        "profile lock must apply to API users too"
    );
    let view = browser
        .open(&format!(
            "file://{}",
            root.join("examples/smoke.html").display()
        ))
        .unwrap();
    until(
        &mut browser,
        |e| matches!(e, BrowserEvent::Loaded { view: v, .. } if *v == view),
    );
    frame(&mut browser, view, 800, 600);
    assert_eq!(browser.view(view).unwrap().title, "Bug renderer smoke test");
    browser
        .resize(
            view,
            Viewport {
                width: 640,
                height: 480,
                scale: 1.,
            },
        )
        .unwrap();
    let pixels = frame(&mut browser, view, 640, 480);
    pixels.write_ppm(root.join("build/api-smoke.ppm")).unwrap();
    assert!(
        browser
            .resize(
                view,
                Viewport {
                    width: 0,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(browser.view(view).unwrap().viewport.width, 640);
    assert!(browser.set_zoom(view, f64::NAN).is_err());
    browser.set_zoom(view, 1.2).unwrap();
    assert_eq!(browser.view(view).unwrap().zoom, 1.2);
    browser.set_zoom(view, 1.).unwrap();
    browser.set_focused(view, true).unwrap();
    browser
        .eval(
            view,
            "document.querySelector('input').focus(); document.title='input ready'",
        )
        .unwrap();
    until(
        &mut browser,
        |e| matches!(e, BrowserEvent::Title { title, .. } if title == "input ready"),
    );
    for pressed in [true, false] {
        browser
            .key(
                view,
                KeyInput {
                    pressed,
                    name: "X",
                    codepoint: 'x' as u32,
                    modifiers: 0,
                    repeat: false,
                },
            )
            .unwrap();
    }
    until(
        &mut browser,
        |e| matches!(e, BrowserEvent::Title { title, .. } if title == "Bug input: x"),
    );
    browser
        .set_load_script(view, "document.title='API load hook'")
        .unwrap();
    browser.reload(view).unwrap();
    until(
        &mut browser,
        |e| matches!(e, BrowserEvent::Title { title, .. } if title == "API load hook"),
    );
    let second = browser.open("about:blank").unwrap();
    assert_eq!(browser.views().count(), 2);
    browser.close(second).unwrap();
    until(
        &mut browser,
        |e| matches!(e, BrowserEvent::Closed { view } if *view == second),
    );
    assert!(browser.view(second).is_err());
    assert!(browser.read_frame(second, &mut Frame::default()).is_err());
    assert_eq!(browser.views().count(), 1);
    let request = browser.eval(view, "Promise.resolve({answer:42})").unwrap();
    until(&mut browser, |event| {
        if let BrowserEvent::Evaluated {
            request: id,
            result,
            ..
        } = event
            && *id == request
        {
            assert_eq!(
                result,
                &EvaluationResult::Json {
                    value: serde_json::json!({"answer":42})
                }
            );
            return true;
        }
        false
    });
    browser
        .set_metadata(
            view,
            ViewMetadata {
                name: Some("work".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(browser.find_view("work"), Some(view));
    let snapshot: SessionSnapshot =
        serde_json::from_str(&serde_json::to_string(&browser.snapshot()).unwrap()).unwrap();
    assert!(
        browser.restore(&snapshot).is_err(),
        "duplicate stable names must not partially restore"
    );
    browser.close(view).unwrap();
    until(&mut browser, |e| matches!(e, BrowserEvent::Closed { .. }));
    assert_eq!(browser.views().count(), 0);
    let restored = browser.restore(&snapshot).unwrap()[0];
    assert_ne!(restored, view);
    assert_eq!(browser.find_view("work"), Some(restored));
    until(
        &mut browser,
        |event| matches!(event, BrowserEvent::Title { view: id, title } if *id == restored && title == "API load hook"),
    );
    browser.close(restored).unwrap();
    until(&mut browser, |event| {
        matches!(event, BrowserEvent::Closed { .. })
    });
    drop(browser);
    std::fs::remove_dir_all(profile).unwrap();
}

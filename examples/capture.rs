//! A custom headless client: no JSON transport, winit, or built-in client loop.
//! cargo run --no-default-features --example capture -- URL OUTPUT.ppm PROFILE
use bug::{Browser, BrowserEvent, BrowserOptions, Frame, Viewport};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let url = args.next().ok_or("usage: capture URL OUTPUT.ppm PROFILE")?;
    let output = PathBuf::from(args.next().ok_or("missing output path")?);
    let profile = PathBuf::from(args.next().ok_or("missing profile directory")?);
    let mut browser = Browser::new(BrowserOptions {
        profile,
        viewport: Viewport {
            width: 1024,
            height: 768,
            scale: 1.,
        },
        ..Default::default()
    })?;
    let view = browser.open(&url)?;
    let mut loaded = false;
    let mut painted_after_load = false;
    let mut frame = Frame::default();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        browser.poll();
        for event in browser.drain_events() {
            match event {
                BrowserEvent::Loaded { view: id, .. } if id == view => loaded = true,
                BrowserEvent::FrameReady { view: id } if id == view && loaded => {
                    painted_after_load = true
                }
                BrowserEvent::Error { message } => return Err(message),
                _ => {}
            }
        }
        if painted_after_load
            && browser.read_frame(view, &mut frame)?
            && frame.width == 1024
            && frame.height == 768
        {
            frame.write_ppm(&output)?;
            println!(
                "Saved {}x{} to {}",
                frame.width,
                frame.height,
                output.display()
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    Err("timed out waiting for a rendered page".into())
}

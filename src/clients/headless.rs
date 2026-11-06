//! Offscreen views with full layout/rendering, no OS windows or display server.
use super::{Controller, RunOptions, Session};
use crate::{Browser, BrowserEvent};
use std::time::Duration;

pub fn run(
    browser: Browser,
    controller: impl Controller,
    options: RunOptions,
) -> Result<(), String> {
    let mut session = Session::new(browser, controller, options)?;
    session.start()?;
    let mut focused = None;
    while !session.quit {
        for event in session.step() {
            match event {
                BrowserEvent::FocusRequested { view } => {
                    if session.browser.view(view).is_err() {
                        continue;
                    }
                    if let Some(old) = focused
                        && session.browser.view(old).is_ok()
                    {
                        session.browser.set_focused(old, false)?;
                    }
                    session.browser.set_focused(view, true)?;
                    focused = Some(view);
                }
                BrowserEvent::Closed { view } if focused == Some(view) => focused = None,
                _ => {}
            }
        }
        if !session.quit {
            std::thread::sleep(Duration::from_millis(8));
        }
    }
    session.result()
}

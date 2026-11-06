//! Replaceable drivers of the Browser API. Neither requires the JSON protocol.
use crate::{Browser, BrowserEvent, Viewport};
use std::time::{Duration, Instant};

#[cfg(feature = "windowed")]
mod clipboard;
pub mod headless;
#[cfg(feature = "windowed")]
mod palette;
#[cfg(feature = "windowed")]
mod shortcuts;
#[cfg(feature = "windowed")]
pub mod windowed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Continue,
    Quit,
}

/// An optional source of browser operations and consumer of events. Implement
/// this for your own controller; the JSON adapter is one implementation.
pub trait Controller {
    fn started(&mut self) {}
    fn update(&mut self, _browser: &mut Browser) -> Result<Control, String> {
        Ok(Control::Continue)
    }
    fn event(&mut self, _event: &BrowserEvent) {}
}
impl Controller for () {}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub session: Option<crate::SessionSnapshot>,
    pub urls: Vec<String>,
    pub viewport: Viewport,
    pub timeout: Option<Duration>,
    pub exit_after_load: bool,
}
impl Default for RunOptions {
    fn default() -> Self {
        Self {
            session: None,
            urls: vec!["about:blank".into()],
            viewport: Viewport::default(),
            timeout: None,
            exit_after_load: false,
        }
    }
}

pub(super) struct Session<C> {
    pub browser: Browser,
    pub controller: C,
    pub options: RunOptions,
    pub quit: bool,
    pub failed: bool,
    deadline: Option<Instant>,
    pending_loads: std::collections::HashSet<crate::ViewId>,
}
impl<C: Controller> Session<C> {
    fn new(mut browser: Browser, controller: C, options: RunOptions) -> Result<Self, String> {
        browser.set_default_viewport(options.viewport)?;
        let deadline = options.timeout.map(|duration| Instant::now() + duration);
        Ok(Self {
            browser,
            controller,
            options,
            quit: false,
            failed: false,
            deadline,
            pending_loads: Default::default(),
        })
    }
    fn start(&mut self) -> Result<(), String> {
        self.controller.started();
        if let Some(snapshot) = &self.options.session {
            self.pending_loads.extend(self.browser.restore(snapshot)?);
        }
        for url in &self.options.urls {
            self.pending_loads.insert(self.browser.open(url)?);
        }
        Ok(())
    }
    fn step(&mut self) -> Vec<BrowserEvent> {
        match self.controller.update(&mut self.browser) {
            Ok(Control::Quit) => self.quit = true,
            Err(message) => self.report_error(message),
            _ => {}
        }
        self.browser.poll();
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.quit = true;
        }
        self.events()
    }
    fn events(&mut self) -> Vec<BrowserEvent> {
        let events: Vec<_> = self.browser.drain_events().collect();
        for event in &events {
            match event {
                BrowserEvent::Closed { view } => {
                    self.pending_loads.remove(view);
                    if self.browser.views().next().is_none() {
                        self.quit = true;
                    }
                }
                BrowserEvent::Loaded { view, .. } => {
                    self.pending_loads.remove(view);
                    if self.options.exit_after_load && self.pending_loads.is_empty() {
                        self.quit = true;
                    }
                }
                BrowserEvent::Error { .. } => self.failed = true,
                _ => {}
            }
            self.controller.event(event);
        }
        events
    }
    fn report_error(&mut self, message: String) {
        self.controller.event(&BrowserEvent::Error { message });
        self.failed = true;
    }
    fn result(&self) -> Result<(), String> {
        if self.failed {
            Err("one or more browser operations failed".into())
        } else {
            Ok(())
        }
    }
}

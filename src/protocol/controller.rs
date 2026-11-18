use super::{Command, Event, Notification, output::Output};
use crate::{
    Browser, BrowserEvent,
    clients::{Control, Controller},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::{BufRead, Read, Write},
    marker::PhantomData,
    rc::Rc,
    sync::mpsc::{self, Receiver},
};

/// A JSON stream driven by a browser loop. Input and output run on worker threads.
pub struct JsonController<W> {
    incoming: Receiver<Result<Value, String>>,
    output: Output,
    notifications: Option<Vec<Notification>>,
    delegated: HashSet<u64>,
    requests: HashSet<(u64, u64)>,
    owners: Rc<RefCell<HashSet<u64>>>,
    evaluations: HashMap<u64, u64>,
    loads: HashMap<u64, u64>,
    eof: bool,
    peer: bool,
    writer: PhantomData<W>,
}

impl<W: Write + Send + 'static> JsonController<W> {
    pub fn new(mut input: impl BufRead + Send + 'static, writer: W) -> Self {
        let (sender, incoming) = mpsc::sync_channel(64);
        std::thread::spawn(move || {
            loop {
                let mut line = String::new();
                let record = match (&mut input).take(1024 * 1024 + 1).read_line(&mut line) {
                    Ok(0) => break,
                    Err(e) => {
                        let _ = sender.send(Err(e.to_string()));
                        break;
                    }
                    Ok(_) if line.len() > 1024 * 1024 => {
                        if !line.ends_with('\n') {
                            let _ = input.skip_until(b'\n');
                        }
                        Err("command exceeds 1 MiB".into())
                    }
                    Ok(_) => serde_json::from_str(&line).map_err(|e| e.to_string()),
                };
                if sender.send(record).is_err() {
                    break;
                }
            }
        });
        Self {
            incoming,
            output: Output::new(writer),
            notifications: None,
            delegated: Default::default(),
            requests: Default::default(),
            owners: Default::default(),
            evaluations: Default::default(),
            loads: Default::default(),
            eof: false,
            peer: false,
            writer: PhantomData,
        }
    }
    /// A second connection to the same browser, initially listening only for replies.
    pub fn peer<T: Write + Send + 'static>(
        &self,
        input: impl BufRead + Send + 'static,
        writer: T,
    ) -> JsonController<T> {
        let mut peer = JsonController::new(input, writer);
        peer.owners = self.owners.clone();
        peer.notifications = Some(Vec::new());
        peer.peer = true;
        peer
    }
    pub fn disconnected(&self) -> bool {
        self.eof || self.output.disconnected()
    }
    /// Release pending native callbacks before removing this connection.
    pub fn disconnect(&mut self, browser: &mut Browser) {
        self.eof = true;
        for view in self.delegated.drain() {
            self.owners.borrow_mut().remove(&view);
            let _ = browser.set_request_handlers(view, &[]);
        }
    }
    fn execute(
        &mut self,
        mut wire: Value,
        id: Option<u64>,
        browser: &mut Browser,
    ) -> Result<Control, String> {
        let object = wire.as_object_mut().ok_or("expected a command object")?;
        object.remove("request_id");
        let wait = match object.remove("wait_for_load") {
            None => false,
            Some(Value::Bool(wait))
                if id.is_some() && object.get("cmd").and_then(Value::as_str) == Some("open") =>
            {
                wait
            }
            _ => return Err("wait_for_load requires open with a request_id".into()),
        };
        if let Some(selector) = object.get("view").and_then(Value::as_str) {
            let view = match selector {
                "focused" => browser.focused_view(),
                "last_focused" => browser.last_focused_view(),
                _ => return Err(format!("invalid view selector {selector}")),
            }
            .ok_or_else(|| format!("no {selector} view"))?;
            object.insert("view".into(), view.into());
        }
        if let Some(view) = object.get("view").and_then(Value::as_u64) {
            browser.view(view)?;
        }
        if id.is_some_and(|id| {
            self.evaluations
                .values()
                .chain(self.loads.values())
                .any(|v| *v == id)
        }) {
            return Err("request_id already pending".into());
        }
        if wait && self.loads.len() >= 128 {
            return Err("too many pending loads".into());
        }
        let command: Command = serde_json::from_value(wire).map_err(|e| e.to_string())?;
        command.validate()?;
        let mut registration = None;
        match &command {
            Command::SetRequestHandlers { view, kinds } => {
                if self.owners.borrow().contains(view) && !self.delegated.contains(view) {
                    return Err("another controller owns this view's handlers".into());
                }
                registration = Some((*view, !kinds.is_empty()));
            }
            Command::ReplyRequest { view, .. } | Command::ReplyFilePicker { view, .. }
                if self.owners.borrow().contains(view) && !self.delegated.contains(view) =>
            {
                return Err("another controller owns this view's handlers".into());
            }
            _ => {}
        }
        let answering = match &command {
            Command::ReplyRequest { view, request, .. }
            | Command::ReplyFilePicker { view, request, .. } => Some((*view, *request)),
            _ => None,
        };
        let (control, event) = command.execute(browser)?;
        if let Some(key) = answering {
            self.requests.remove(&key);
        }
        if let Some((view, register)) = registration {
            if register {
                self.delegated.insert(view);
                self.owners.borrow_mut().insert(view);
            } else {
                self.delegated.remove(&view);
                self.owners.borrow_mut().remove(&view);
            }
        }
        if let Some(Event::Subscribed { events }) = &event {
            self.notifications.clone_from(events);
        }
        if let Some(id) = id {
            match event {
                Some(Event::Opened { view }) if wait => {
                    self.loads.insert(view, id);
                }
                Some(Event::EvaluationQueued { request, .. }) => {
                    self.evaluations.insert(request, id);
                }
                other => {
                    let mut result = serde_json::to_value(other).map_err(|e| e.to_string())?;
                    if let Some(obj) = result.as_object_mut() {
                        obj.remove("event");
                    }
                    self.output.send(&json!({"reply_to":id,"result":result}));
                }
            }
        } else if let Some(event) = event
            && !matches!(event, Event::Opened { .. })
        {
            self.output.send(&event);
        }
        Ok(control)
    }
}

impl<W: Write + Send + 'static> Controller for JsonController<W> {
    fn started(&mut self) {
        self.output.send(&Event::Ready { protocol: 1 });
    }
    fn update(&mut self, browser: &mut Browser) -> Result<Control, String> {
        if self.disconnected() {
            self.disconnect(browser);
            return Ok(Control::Continue);
        }
        for _ in 0..64 {
            let record = match self.incoming.try_recv() {
                Ok(value) => value,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.eof = true;
                    self.disconnect(browser);
                    break;
                }
            };
            let id = record
                .as_ref()
                .ok()
                .and_then(|v| v.get("request_id"))
                .and_then(Value::as_u64);
            let result = record.and_then(|wire| {
                if id.is_none() && wire.get("request_id").is_some() {
                    return Err("request_id must be an unsigned integer".into());
                }
                if self.peer && id.is_none() {
                    return Err("socket commands require a request_id".into());
                }
                self.execute(wire, id, browser)
            });
            match result {
                Ok(Control::Quit) => return Ok(Control::Quit),
                Ok(Control::Continue) => {}
                Err(message) => match id {
                    Some(id) => self
                        .output
                        .send(&json!({"reply_to": id,"error":{"message": message}})),
                    None => self.output.send(&Event::Error { message }),
                },
            }
            if self.output.disconnected() {
                self.disconnect(browser);
                break;
            }
        }
        Ok(Control::Continue)
    }
    fn event(&mut self, event: &BrowserEvent) {
        if self.output.disconnected() {
            return;
        }
        match event {
            BrowserEvent::Evaluated {
                request, result, ..
            } => {
                if let Some(id) = self.evaluations.remove(request) {
                    match result {
                        crate::EvaluationResult::Error { .. } => {
                            self.output.send(&json!({"reply_to":id,"error":result}))
                        }
                        _ => self.output.send(&json!({"reply_to":id,"result":result})),
                    }
                    return;
                }
                if self.peer {
                    return;
                }
            }
            BrowserEvent::Loaded { view, .. } => {
                if let Some(id) = self.loads.remove(view) {
                    self.output
                        .send(&json!({"reply_to":id,"result":{"view":view}}));
                }
            }
            BrowserEvent::Closed { view } => {
                self.delegated.remove(view);
                self.owners.borrow_mut().remove(view);
                if let Some(id) = self.loads.remove(view) {
                    self.output.send(
                        &json!({"reply_to":id,"error":{"message":"view closed before load"}}),
                    );
                }
            }
            BrowserEvent::Request { view, callback } => {
                if !self.delegated.contains(view) {
                    return;
                }
                self.requests.insert((*view, callback.request));
            }
            BrowserEvent::RequestCancelled { view, request } => {
                if !self.requests.remove(&(*view, *request)) && self.peer {
                    return;
                }
            }
            BrowserEvent::FilePickerRequested { .. } | BrowserEvent::Error { .. } if self.peer => {
                return;
            }
            _ => {}
        }
        if let Some(event) = Event::from_browser(event)
            && !matches!((&self.notifications, event.notification()), (Some(allowed), Some(kind)) if !allowed.contains(&kind))
        {
            self.output.send(&event);
        }
    }
}

use bug::commands::{ACTIONS, Action, Description};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
};

pub fn help(topic: Option<&str>) -> Result<(), String> {
    if let Some(name) = topic {
        if name == "socket" {
            println!("bug socket\nPrint the running profile’s local control socket path.");
        } else if name == "watch" {
            println!(
                "bug watch\nStream view notifications as newline-delimited JSON until interrupted."
            );
        } else {
            let entry = find(name).ok_or_else(|| format!("unknown command {name}"))?;
            println!("bug {} {}\n{}", entry.name, entry.usage, entry.help);
            if entry.action.takes_target() {
                println!("TARGET is --view ID, --focused (default), or --last-focused.");
            }
        }
        println!("Put --profile DIR before the command. Errors go to stderr and exit 1.");
        return Ok(());
    }
    println!(
        "Usage: bug [--profile DIR] [URL ...]\n       bug [--profile DIR] COMMAND [ARGS]\n\nA URL opens a window in the selected profile, starting Bug if needed.\nCommands address a running browser. Use bug help COMMAND for arguments.\n"
    );
    for command in ACTIONS {
        println!("  {:12} {}", command.name, command.label);
    }
    println!(
        "  socket       Print control socket path\n  watch        Stream view notifications\n\nHost options: --headless, --size WIDTHxHEIGHT, --engine PATH, --session FILE,\n              --exit-after-load, --exit-after-ms N, --version\nJSON commands on stdin and events on stdout. Ctrl+Shift+P opens the palette."
    );
    Ok(())
}
pub fn find(name: &str) -> Option<&'static Description> {
    ACTIONS.iter().find(|e| e.name == name)
}

pub struct Call {
    wire: Value,
    kind: Option<Action>,
    json: bool,
}
impl Call {
    pub fn open(url: &str) -> Self {
        Self {
            wire: json!({"cmd":"open","url":url,"request_id":1}),
            kind: Some(Action::Open),
            json: false,
        }
    }
    pub fn parse(name: &str, args: Vec<String>) -> Result<Self, String> {
        if name == "watch" && args.is_empty() {
            return Ok(Self {
                wire: json!({"cmd":"subscribe","events":null,"request_id":1}),
                kind: None,
                json: true,
            });
        }
        let entry = find(name).ok_or_else(|| format!("unknown command or arguments for {name}"))?;
        let mut wire = json!({"cmd":name.replace('-',"_"),"request_id":1});
        let mut args = args.into_iter().peekable();
        let mut target = None;
        let mut as_json = false;
        while let Some(flag) = args.peek().filter(|s| s.starts_with('-')).cloned() {
            args.next();
            match flag.as_str() {
                "--" => break,
                "--wait" if entry.action == Action::Open => wire["wait_for_load"] = true.into(),
                "--json" if matches!(entry.action, Action::Views | Action::Eval) => as_json = true,
                "--view" | "--focused" | "--last-focused"
                    if entry.action.takes_target() && target.is_none() =>
                {
                    target = Some(match flag.as_str() {
                        "--view" => args
                            .next()
                            .and_then(|s| s.parse::<u64>().ok())
                            .ok_or("--view requires a numeric view ID")?
                            .into(),
                        "--focused" => "focused".into(),
                        _ => "last_focused".into(),
                    });
                }
                _ => {
                    return Err(format!(
                        "invalid option {flag} for {name}; see bug help {name}"
                    ));
                }
            }
        }
        if entry.action.takes_target() {
            wire["view"] = target.unwrap_or_else(|| "focused".into());
        }
        match entry.action {
            Action::Open | Action::Navigate => {
                wire["url"] = args.next().ok_or("missing URL")?.into()
            }
            Action::Eval => wire["script"] = args.next().ok_or("missing script")?.into(),
            Action::Views => wire["cmd"] = "list".into(),
            _ => {}
        }
        if args.next().is_some() {
            return Err(format!("too many arguments; see bug help {name}"));
        }
        Ok(Self {
            wire,
            kind: Some(entry.action),
            json: as_json,
        })
    }
    pub fn run(&self, mut socket: UnixStream, quiet: bool) -> Result<(), String> {
        let mut request = serde_json::to_vec(&self.wire).map_err(|e| e.to_string())?;
        request.push(b'\n');
        if request.len() > 1024 * 1024 {
            return Err("command exceeds 1 MiB".into());
        }
        socket.write_all(&request).map_err(|e| e.to_string())?;
        let input = BufReader::new(socket);
        let mut acknowledged = false;
        for line in input.lines() {
            let line = line.map_err(|e| e.to_string())?;
            let reply: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if reply["reply_to"] != 1 {
                if reply["event"] == "error" {
                    return Err(reply["message"].as_str().unwrap_or("protocol error").into());
                }
                if self.kind.is_none()
                    && acknowledged
                    && let Err(e) = writeln!(std::io::stdout().lock(), "{line}")
                {
                    if e.kind() == std::io::ErrorKind::BrokenPipe {
                        return Ok(());
                    }
                    return Err(e.to_string());
                }
                continue;
            }
            if let Some(error) = reply.get("error") {
                return Err(error["message"].as_str().unwrap_or("command failed").into());
            }
            let value = &reply["result"];
            if self.kind.is_none() {
                acknowledged = true;
                continue;
            }
            if !quiet {
                let mut stdout = std::io::stdout().lock();
                let printed = match self.kind {
                    Some(Action::Open) => writeln!(stdout, "{}", value["view"]),
                    Some(Action::Views) => writeln!(stdout, "{}", value["views"]),
                    Some(Action::Eval) if self.json => writeln!(stdout, "{value}"),
                    Some(Action::Eval) => match &value["value"] {
                        Value::String(text) => writeln!(stdout, "{text}"),
                        _ if value["type"] == "undefined" => writeln!(stdout, "undefined"),
                        other => writeln!(stdout, "{other}"),
                    },
                    _ => Ok(()),
                };
                if let Err(error) = printed
                    && error.kind() != std::io::ErrorKind::BrokenPipe
                {
                    return Err(error.to_string());
                }
            }
            return Ok(());
        }
        if self.kind.is_none() && acknowledged {
            return Err("event stream disconnected".into());
        }
        Err("Bug disconnected before replying".into())
    }
}

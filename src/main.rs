mod cli;
mod instance;
use bug::{
    Browser, BrowserOptions,
    clients::{self, RunOptions},
};
use std::{
    io,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn connect(profile: &Path) -> io::Result<UnixStream> {
    UnixStream::connect(instance::address(profile)?)
}

fn run() -> Result<(), String> {
    let mut browser_options = BrowserOptions::default();
    let mut options = RunOptions::default();
    options.urls.clear();
    let mut headless = false;
    let mut host_options = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if matches!(
            arg.as_str(),
            "--headless"
                | "--size"
                | "--session"
                | "--engine"
                | "--exit-after-ms"
                | "--exit-after-load"
        ) {
            host_options = true;
        }
        match arg.as_str() {
            "--" => {
                options.urls.extend(args);
                break;
            }
            "help" if options.urls.is_empty() => return cli::help(args.next().as_deref()),
            "socket" if options.urls.is_empty() => {
                if let Some(arg) = args.next() {
                    if arg == "--help" || arg == "-h" {
                        return cli::help(Some("socket"));
                    }
                    return Err("usage: bug [--profile DIR] socket".into());
                }
                let path =
                    instance::address(&browser_options.profile).map_err(|e| e.to_string())?;
                UnixStream::connect(&path).map_err(|e| format!("cannot reach running Bug: {e}"))?;
                println!("{}", path.display());
                return Ok(());
            }
            name if options.urls.is_empty() && (cli::find(name).is_some() || name == "watch") => {
                let rest: Vec<_> = args.collect();
                if rest == ["--help"] || rest == ["-h"] {
                    return cli::help(Some(name));
                }
                if host_options {
                    return Err(
                        "host options cannot be used with a running-instance command".into(),
                    );
                }
                let call = cli::Call::parse(name, rest)?;
                let stream = connect(&browser_options.profile).map_err(|e| {
                    format!(
                        "cannot reach Bug for profile {}: {e}",
                        browser_options.profile.display()
                    )
                })?;
                return call.run(stream, false);
            }
            "--version" | "-V" => {
                println!("bug {} ({})", bug::VERSION, bug::COMMIT);
                return Ok(());
            }
            "--help" | "-h" => return cli::help(None),
            "--headless" => headless = true,
            "--session" => {
                options.session = Some(bug::protocol::read_session(
                    args.next().ok_or("--session requires a path")?,
                )?)
            }
            "--engine" => {
                browser_options.engine =
                    PathBuf::from(args.next().ok_or("--engine requires a path")?)
            }
            "--profile" => {
                browser_options.profile =
                    PathBuf::from(args.next().ok_or("--profile requires a directory")?)
            }
            "--size" => {
                let size = args.next().ok_or("--size requires WIDTHxHEIGHT")?;
                let (width, height) = size.split_once('x').ok_or("--size requires WIDTHxHEIGHT")?;
                options.viewport.width = width.parse().map_err(|_| "invalid width")?;
                options.viewport.height = height.parse().map_err(|_| "invalid height")?;
            }
            "--exit-after-ms" => {
                let ms: u64 = args
                    .next()
                    .ok_or("--exit-after-ms requires milliseconds")?
                    .parse()
                    .map_err(|_| "invalid milliseconds")?;
                options.timeout = Some(Duration::from_millis(ms));
            }
            "--exit-after-load" => options.exit_after_load = true,
            _ if arg.starts_with('-') => return Err(format!("unknown option {arg}")),
            _ => options.urls.push(arg),
        }
    }
    if options.urls.is_empty() && options.session.is_none() {
        options.urls.push("about:blank".into());
    }
    options.viewport.validate()?;
    let host = match instance::Host::bind(&browser_options.profile) {
        Ok(host) => host,
        Err(e) if e.kind() == io::ErrorKind::WouldBlock && !host_options => {
            // A simultaneous first launch may still be binding its socket.
            let deadline = Instant::now() + Duration::from_secs(2);
            for url in &options.urls {
                let stream = loop {
                    match connect(&browser_options.profile) {
                        Ok(stream) => break stream,
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                            ) && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(e) => return Err(format!("cannot reach running Bug: {e}")),
                    }
                };
                cli::Call::open(url).run(stream, true)?;
            }
            return Ok(());
        }
        Err(e) => {
            return Err(format!(
                "cannot start Bug for profile {}: {e}",
                browser_options.profile.display()
            ));
        }
    };
    #[cfg(not(feature = "windowed"))]
    if !headless {
        return Err(
            "this build has no windowed client; pass --headless or build with the windowed feature"
                .into(),
        );
    }
    let browser = Browser::new(browser_options)?;
    let controller = instance::Instance::new(host);
    if headless {
        return clients::headless::run(browser, controller, options);
    }
    #[cfg(feature = "windowed")]
    return clients::windowed::run(browser, controller, options);
    #[cfg(not(feature = "windowed"))]
    unreachable!()
}
fn main() {
    if let Err(e) = run() {
        eprintln!("bug: {e}");
        std::process::exit(1);
    }
}

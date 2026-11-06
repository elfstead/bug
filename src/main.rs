use bug::{
    Browser, BrowserOptions,
    clients::{self, RunOptions},
    protocol::JsonController,
};
use std::{io, path::PathBuf, time::Duration};

fn run() -> Result<(), String> {
    let mut browser_options = BrowserOptions::default();
    let mut options = RunOptions::default();
    options.urls.clear();
    let mut headless = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("bug {} ({})", bug::VERSION, bug::COMMIT);
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "bug [--headless] [--size WIDTHxHEIGHT] [--engine PATH] [--profile DIR]\n    [--session FILE] [--exit-after-ms N] [--exit-after-load] [URL ...]\n    --version: print build identity without loading the engine\n\nJSON commands on stdin; JSON events on stdout.\nCtrl+Shift+P: command palette. Ctrl+C/X/V: text clipboard. No Qt.\nHeadless views render without X11 or Wayland. See README.md for the Rust API."
                );
                return Ok(());
            }
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
    #[cfg(not(feature = "windowed"))]
    if !headless {
        return Err(
            "this build has no windowed client; pass --headless or build with the windowed feature"
                .into(),
        );
    }
    let browser = Browser::new(browser_options)?;
    let controller = JsonController::new(io::BufReader::new(io::stdin()), io::stdout());
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

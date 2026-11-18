//! The CLI rendezvous is local to one Unix account and one profile inode.
//! The host lock serializes stale-socket recovery and simultaneous launches.
use bug::{
    Browser, BrowserEvent,
    clients::{Control, Controller},
    protocol::JsonController,
};
use std::{
    fs::{self, DirBuilder, File},
    io::{self, BufReader},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
};

pub fn address(profile: &Path) -> io::Result<PathBuf> {
    let meta = profile.metadata()?;
    // A short address also works with deeply nested profiles on macOS.
    let uid = rustix::process::geteuid().as_raw();
    let directory = PathBuf::from(format!("/tmp/bug-{uid}"));
    match DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let dir = directory.symlink_metadata()?;
    if !dir.is_dir() || dir.uid() != uid || dir.mode() & 0o077 != 0 {
        return Err(io::Error::other("unsafe Bug socket directory"));
    }
    Ok(directory.join(format!("{:x}-{:x}.sock", meta.dev(), meta.ino())))
}

pub struct Host {
    listener: UnixListener,
    address: PathBuf,
    _lock: File,
}

impl Host {
    pub fn bind(profile: &Path) -> io::Result<Self> {
        fs::create_dir_all(profile)?;
        let address = address(profile)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(profile.join(".bug-host.lock"))?;
        lock.try_lock().map_err(io::Error::from)?;
        match fs::remove_file(&address) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&address)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            address,
            _lock: lock,
        })
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.address);
    }
}

struct Peer {
    controller: JsonController<UnixStream>,
    socket: UnixStream,
}
impl Drop for Peer {
    fn drop(&mut self) {
        // Unblock both workers, including a writer stalled on a full kernel buffer.
        // An orderly host shutdown flushes queued replies before closing the socket.
        let _ = self.socket.shutdown(std::net::Shutdown::Read);
        if self.controller.disconnected() {
            let _ = self.socket.shutdown(std::net::Shutdown::Write);
        }
    }
}

pub struct Instance {
    host: Host,
    stdio: JsonController<io::Stdout>,
    peers: Vec<Peer>,
}
impl Instance {
    pub fn new(host: Host) -> Self {
        Self {
            host,
            stdio: JsonController::new(BufReader::new(io::stdin()), io::stdout()),
            peers: Vec::new(),
        }
    }
}
impl Controller for Instance {
    fn started(&mut self) {
        self.stdio.started();
    }
    fn update(&mut self, browser: &mut Browser) -> Result<Control, String> {
        for _ in 0..8 {
            match self.host.listener.accept() {
                Ok((stream, _)) if self.peers.len() < 32 => {
                    // Accepted sockets may inherit O_NONBLOCK on BSD systems.
                    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
                    let reader = stream.try_clone().map_err(|e| e.to_string())?;
                    let writer = stream.try_clone().map_err(|e| e.to_string())?;
                    self.peers.push(Peer {
                        controller: self.stdio.peer(BufReader::new(reader), writer),
                        socket: stream,
                    });
                }
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        if self.stdio.update(browser)? == Control::Quit {
            return Ok(Control::Quit);
        }
        for peer in &mut self.peers {
            if peer.controller.update(browser)? == Control::Quit {
                return Ok(Control::Quit);
            }
        }
        self.peers.retain_mut(|p| {
            if p.controller.disconnected() {
                p.controller.disconnect(browser);
                false
            } else {
                true
            }
        });
        Ok(Control::Continue)
    }
    fn event(&mut self, event: &BrowserEvent) {
        self.stdio.event(event);
        for peer in &mut self.peers {
            peer.controller.event(event);
        }
    }
}

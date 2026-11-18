//! Never wait for a pipe reader on the browser thread.
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::Duration,
};

const BYTE_LIMIT: usize = 16 * 1024 * 1024;

pub(super) struct Output {
    sender: Option<SyncSender<Vec<u8>>>,
    queued: Arc<AtomicUsize>,
    failed: Arc<AtomicBool>,
    finished: Receiver<()>,
}
impl Output {
    pub fn new(mut writer: impl Write + Send + 'static) -> Self {
        let (sender, records) = sync_channel::<Vec<u8>>(256);
        let (done, finished) = sync_channel(1);
        let queued = Arc::new(AtomicUsize::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let count = queued.clone();
        let failure = failed.clone();
        std::thread::spawn(move || {
            for record in records {
                if writer
                    .write_all(&record)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    failure.store(true, Ordering::Release);
                    break;
                }
                count.fetch_sub(record.len(), Ordering::Relaxed);
            }
            // If a stalled stdout reader resumes, make the lost stream visible.
            // A socket peer may already have been shut down.
            if failure.load(Ordering::Acquire) {
                let _ = writer.write_all(
                    b"{\"event\":\"error\",\"message\":\"controller output disconnected\"}\n",
                );
                let _ = writer.flush();
            }
            let _ = done.send(());
        });
        Self {
            sender: Some(sender),
            queued,
            failed,
            finished,
        }
    }
    pub fn disconnected(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
    pub fn send(&mut self, value: &impl serde::Serialize) {
        if self.disconnected() {
            return;
        }
        let mut bytes = serde_json::to_vec(value).expect("serializable protocol output");
        bytes.push(b'\n');
        let len = bytes.len();
        if self
            .queued
            .fetch_add(len, Ordering::Relaxed)
            .saturating_add(len)
            > BYTE_LIMIT
            || self.sender.as_ref().unwrap().try_send(bytes).is_err()
        {
            self.failed.store(true, Ordering::Release);
            self.sender.take();
        }
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        self.sender.take();
        // Allow ordinary replies (including quit) to reach a healthy reader at shutdown.
        if !self.disconnected() {
            let _ = self.finished.recv_timeout(Duration::from_millis(100));
        }
    }
}

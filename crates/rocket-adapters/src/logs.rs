//! Service and job log files with an in-memory ring buffer per log, publishing
//! new lines on the event bus (Go: `adapters/logs`).
//!
//! Children write straight into the log file (no pipe through the daemon), so
//! they keep running and logging even if the daemon restarts. Each opened log
//! is followed by a polling thread (same 200ms tick as Go) that turns appended
//! bytes into lines.

use rocket_domain::ports::{EventBus, LogSink, Result};
use rocket_domain::{Event, event_type};
use std::collections::HashMap;
use std::fs::File;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;
use time::OffsetDateTime;

/// Lines kept in memory per followed log.
const RING_SIZE: usize = 2000;
/// Polling tick of the followers.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// At most this many bytes are consumed per poll.
const POLL_CHUNK: u64 = 4 << 20;
/// Tail reads look at most this far back from the end of the file.
const TAIL_WINDOW: u64 = 1 << 20;

/// [`LogSink`] over per-service log files under one directory.
pub struct Sink {
    dir: PathBuf,
    bus: Option<Arc<dyn EventBus>>,
    ring_size: usize,
    interval: Duration,
    followers: Mutex<HashMap<String, Follower>>,
}

struct Follower {
    ring: Arc<Mutex<Ring>>,
    /// Dropping the sender tells the thread to do a final poll and exit.
    quit: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Follower {
    /// Signals the thread and waits for its final poll to finish.
    fn stop(mut self) {
        drop(self.quit.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Sink {
    /// A sink rooted at `dir` (e.g. `~/.rocket/logs`). `bus` receives
    /// `log.line` / `job.log` events when present.
    pub fn new(dir: impl Into<PathBuf>, bus: Option<Arc<dyn EventBus>>) -> Self {
        Self {
            dir: dir.into(),
            bus,
            ring_size: RING_SIZE,
            interval: POLL_INTERVAL,
            followers: Mutex::new(HashMap::new()),
        }
    }

    /// Overrides the polling tick (tests use a few milliseconds).
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Overrides the number of lines kept in memory per log.
    #[must_use]
    pub fn with_ring_size(mut self, lines: usize) -> Self {
        self.ring_size = lines.max(1);
        self
    }

    /// The log file of a service.
    pub fn path(&self, project: &str, service: &str) -> PathBuf {
        self.dir.join(project).join(format!("{service}.log"))
    }

    /// The log file of a job.
    pub fn job_path(&self, project: &str, job_id: &str) -> PathBuf {
        self.dir
            .join(project)
            .join("jobs")
            .join(format!("{job_id}.log"))
    }

    /// Stops every follower after a final poll.
    pub fn close(&self) {
        let all: Vec<Follower> = self.followers().drain().map(|(_, f)| f).collect();
        for f in all {
            f.stop();
        }
    }

    fn followers(&self) -> MutexGuard<'_, HashMap<String, Follower>> {
        self.followers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn open_log(&self, key: String, path: PathBuf, who: Who) -> Result<(File, String)> {
        if let Some(parent) = path.parent() {
            create_private_dir(parent)?;
        }
        let file = open_append(&path)?;
        let display = path.to_string_lossy().into_owned();
        let mut followers = self.followers();
        if followers.contains_key(&key) {
            return Ok((file, display));
        }
        let offset = file.metadata().map_or(0, |m| m.len());
        let ring = Arc::new(Mutex::new(Ring::new(self.ring_size)));
        let (quit, quit_rx) = mpsc::channel();
        let poller = Poller {
            path,
            who,
            bus: self.bus.clone(),
            ring: Arc::clone(&ring),
            offset,
            partial: Vec::new(),
        };
        let interval = self.interval;
        let thread = std::thread::Builder::new()
            .name("rocket-logs".into())
            .spawn(move || poller.run(&quit_rx, interval))?;
        followers.insert(
            key,
            Follower {
                ring,
                quit: Some(quit),
                thread: Some(thread),
            },
        );
        Ok((file, display))
    }
}

fn job_key(project: &str, job_id: &str) -> String {
    format!("job:{project}/{job_id}")
}

impl LogSink for Sink {
    /// Returns the append-mode log file and starts following it.
    fn open(&self, project: &str, service: &str) -> Result<(File, String)> {
        self.open_log(
            format!("{project}/{service}"),
            self.path(project, service),
            Who::Service {
                project: project.into(),
                service: service.into(),
            },
        )
    }

    /// The last `n` lines: from the ring buffer when it holds enough,
    /// otherwise from the file.
    fn tail(&self, project: &str, service: &str, n: usize) -> Result<Vec<String>> {
        let ring = self
            .followers()
            .get(&format!("{project}/{service}"))
            .map(|f| Arc::clone(&f.ring));
        if let Some(ring) = ring {
            let lines = ring.lock().unwrap_or_else(PoisonError::into_inner).last(n);
            if lines.len() == n {
                return Ok(lines);
            }
        }
        Ok(tail_file(&self.path(project, service), n)?)
    }

    /// [`open`](LogSink::open) for a job log; lines publish as `job.log`.
    fn open_job(&self, project: &str, job_id: &str) -> Result<(File, String)> {
        self.open_log(
            job_key(project, job_id),
            self.job_path(project, job_id),
            Who::Job {
                project: project.into(),
                job_id: job_id.into(),
            },
        )
    }

    fn tail_job(&self, project: &str, job_id: &str, n: usize) -> Result<Vec<String>> {
        Ok(tail_file(&self.job_path(project, job_id), n)?)
    }

    /// Final poll (publishing the remaining lines, including a trailing
    /// partial one), then stops following the job log.
    fn close_job(&self, project: &str, job_id: &str) {
        let follower = self.followers().remove(&job_key(project, job_id));
        if let Some(f) = follower {
            f.stop();
        }
    }

    fn remove_job(&self, project: &str, job_id: &str) -> Result<()> {
        self.close_job(project, job_id);
        match std::fs::remove_file(self.job_path(project, job_id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.close();
    }
}

/// Who a followed log belongs to (decides the published event shape).
enum Who {
    Service { project: String, service: String },
    Job { project: String, job_id: String },
}

struct Poller {
    path: PathBuf,
    who: Who,
    bus: Option<Arc<dyn EventBus>>,
    ring: Arc<Mutex<Ring>>,
    offset: u64,
    /// Bytes after the last newline, carried to the next poll.
    partial: Vec<u8>,
}

impl Poller {
    fn run(mut self, quit: &mpsc::Receiver<()>, interval: Duration) {
        loop {
            match quit.recv_timeout(interval) {
                Err(RecvTimeoutError::Timeout) => self.poll(),
                // Stop requested (sender dropped): drain, flush, exit.
                Ok(()) | Err(RecvTimeoutError::Disconnected) => {
                    self.poll();
                    self.flush_partial();
                    return;
                }
            }
        }
    }

    fn flush_partial(&mut self) {
        if !self.partial.is_empty() {
            let line = lossy_line(&self.partial);
            self.partial.clear();
            self.publish_line(line);
        }
    }

    fn poll(&mut self) {
        let Ok(mut f) = File::open(&self.path) else {
            return;
        };
        let Ok(meta) = f.metadata() else { return };
        if meta.len() < self.offset {
            self.offset = 0; // truncated or rotated
            self.partial.clear();
        }
        if meta.len() == self.offset {
            return;
        }
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        let mut data = Vec::new();
        if f.take(POLL_CHUNK).read_to_end(&mut data).is_err() {
            return;
        }
        self.offset += data.len() as u64;
        let mut text = std::mem::take(&mut self.partial);
        text.extend_from_slice(&data);
        let mut parts = text
            .split(|b| *b == b'\n')
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>();
        self.partial = parts.pop().unwrap_or_default();
        for line in parts {
            self.publish_line(lossy_line(&line));
        }
    }

    fn publish_line(&self, line: String) {
        self.ring
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line.clone());
        let Some(bus) = &self.bus else { return };
        let (r#type, project, service, job_id) = match &self.who {
            Who::Service { project, service } => (
                event_type::LOG_LINE,
                project.clone(),
                service.clone(),
                String::new(),
            ),
            Who::Job { project, job_id } => (
                event_type::JOB_LOG,
                project.clone(),
                String::new(),
                job_id.clone(),
            ),
        };
        bus.publish(Event {
            r#type: r#type.into(),
            time: OffsetDateTime::now_utc(),
            project,
            service,
            state: None,
            line,
            run: None,
            lease: None,
            job_id,
            status: None,
            job: None,
        });
    }
}

/// A line without its `\r` suffix, decoded lossily.
fn lossy_line(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Fixed-size circular buffer of the most recent lines.
struct Ring {
    lines: Vec<String>,
    next: usize,
    full: bool,
}

impl Ring {
    fn new(size: usize) -> Self {
        Self {
            lines: vec![String::new(); size],
            next: 0,
            full: false,
        }
    }

    fn push(&mut self, line: String) {
        self.lines[self.next] = line;
        self.next = (self.next + 1) % self.lines.len();
        if self.next == 0 {
            self.full = true;
        }
    }

    /// The last `n` lines (fewer when the ring holds fewer), oldest first.
    fn last(&self, n: usize) -> Vec<String> {
        let size = if self.full {
            self.lines.len()
        } else {
            self.next
        };
        let n = n.min(size);
        (1..=n)
            .rev()
            .map(|i| self.lines[(self.next + self.lines.len() - i) % self.lines.len()].clone())
            .collect()
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
}

fn open_append(path: &Path) -> std::io::Result<File> {
    let mut o = std::fs::OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// The last `n` lines of the file, looking at most 1 MiB back.
fn tail_file(path: &Path, n: usize) -> std::io::Result<Vec<String>> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    let start = size.saturating_sub(TAIL_WINDOW);
    f.seek(SeekFrom::Start(start))?;
    let mut data = Vec::new();
    f.read_to_end(&mut data)?;
    while data.last() == Some(&b'\n') {
        data.pop();
    }
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines: Vec<&[u8]> = data.split(|b| *b == b'\n').collect();
    if start > 0 && lines.len() > 1 {
        lines.remove(0); // the first line may be cut by the window
    }
    let skip = lines.len().saturating_sub(n);
    Ok(lines[skip..]
        .iter()
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect())
}

/// Emits the last `tail` lines of `path` (`tail < 0`: the whole file), then
/// every new line until `finished()` reports true and the file is drained; a
/// trailing partial line is emitted last. Dropping the future cancels the
/// follow (Go's context cancellation).
pub async fn follow_file<F, E, Fut>(
    path: &Path,
    tail: i64,
    interval: Duration,
    mut finished: F,
    mut emit: E,
) -> Result<()>
where
    F: FnMut() -> bool,
    E: FnMut(String) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let f = File::open(path)?;
    let mut offset = 0u64;
    if let Ok(n) = usize::try_from(tail) {
        let (after, lines) = complete_tail(&f, n)?;
        offset = after;
        for l in lines {
            emit(l).await?;
        }
    }
    let mut partial: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let done = finished(); // checked before reading so nothing written earlier is lost
        loop {
            let (n, err) = match read_at(&f, &mut buf, offset) {
                Ok(n) => (n, false),
                Err(_) => (0, true),
            };
            if n > 0 {
                offset += n as u64;
                partial.extend_from_slice(&buf[..n]);
                let mut parts: Vec<Vec<u8>> =
                    partial.split(|b| *b == b'\n').map(<[u8]>::to_vec).collect();
                partial = parts.pop().unwrap_or_default();
                for l in parts {
                    emit(lossy_line(&l)).await?;
                }
            }
            if err || n < buf.len() {
                break;
            }
        }
        if done {
            if !partial.is_empty() {
                return emit(String::from_utf8_lossy(&partial).into_owned()).await;
            }
            return Ok(());
        }
        tokio::time::sleep(interval).await;
    }
}

/// The last `n` complete lines of `f` (within its last MiB) and the offset
/// just after them; a trailing partial line is left unread.
fn complete_tail(f: &File, n: usize) -> std::io::Result<(u64, Vec<String>)> {
    let size = f.metadata()?.len();
    let start = size.saturating_sub(TAIL_WINDOW);
    let mut data = vec![0u8; (size - start) as usize];
    let mut filled = 0;
    while filled < data.len() {
        let got = read_at(f, &mut data[filled..], start + filled as u64)?;
        if got == 0 {
            break;
        }
        filled += got;
    }
    data.truncate(filled);
    let Some(cut) = data.iter().rposition(|b| *b == b'\n') else {
        return Ok((start, Vec::new()));
    };
    let mut lines: Vec<&[u8]> = data[..cut].split(|b| *b == b'\n').collect();
    if start > 0 && lines.len() > 1 {
        lines.remove(0); // the first line may be cut by the window
    }
    let skip = lines.len().saturating_sub(n);
    let out = lines[skip..]
        .iter()
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect();
    Ok((start + cut as u64 + 1, out))
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(f, buf, offset)
}

#[cfg(windows)]
fn read_at(f: &File, buf: &mut [u8], offset: u64) -> std::io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(f, buf, offset)
}

//! One subprocess's stdio, serviced continuously by its own worker thread.
//!
//! `launch` starts a command in its own process group and runs the caller's
//! protocol future on a dedicated thread's current-thread Tokio runtime,
//! beside one stdout reader, one stderr reader and one stdin writer task,
//! until that future returns. A blocking supervisor thread alone owns the
//! `Child`: it polls for exit and terminates the group, so a stop never waits
//! on the runtime, a pipe or a queue. The transport carries lines; it knows
//! nothing of what they say.
//!
//! Every transient queue is bounded by a `Budget` of entries and payload
//! bytes. An admitted item carries its `Held` allowance until its consumer
//! drops it, so items being written or handled still count.

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::AsyncReadExt;
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use tokio::runtime::Builder;
use tokio::sync::{mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, FramedRead, FramedWrite, LinesCodec, LinesCodecError};

use crate::local_environment::{
    configure_process_group, spawn_process_group, terminate_process_group,
};

/// Fixed internal bounds. Tests pass smaller ones to exercise pressure.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// One stdin or stdout line, excluding its line ending.
    pub(crate) frame_bytes: usize,
    /// Entries in one transient queue.
    pub(crate) queue_entries: usize,
    /// Payload bytes in one transient queue, in-flight entries included.
    pub(crate) queue_bytes: usize,
    /// Retained stderr diagnostics, and the longest record kept.
    pub(crate) stderr_bytes: usize,
    pub(crate) stderr_lines: usize,
}

pub(crate) const LIMITS: Limits = Limits {
    frame_bytes: 16 << 20,
    queue_entries: 128,
    queue_bytes: 32 << 20,
    stderr_bytes: 64 << 10,
    stderr_lines: 50,
};

/// Pending native requests, and live questions, a protocol may hold at once.
pub(crate) const PENDING_LIMIT: usize = 128;

const SUPERVISOR_POLL: Duration = Duration::from_millis(20);
const DRAIN_QUIET: Duration = Duration::from_millis(100);
const DRAIN_CAP: Duration = Duration::from_secs(2);
const RUNTIME_SHUTDOWN: Duration = Duration::from_secs(1);

/// The allowance of one transient queue, in entries and payload bytes.
#[derive(Clone, Debug)]
pub(crate) struct Budget {
    entries: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
    max_bytes: usize,
}

/// One admitted item's share of its queue, returned when dropped.
#[derive(Debug)]
pub(crate) struct Held {
    _entry: OwnedSemaphorePermit,
    _bytes: OwnedSemaphorePermit,
}

/// An item larger than its whole queue, which could never be admitted.
#[derive(Debug)]
pub(crate) struct Oversized;

impl Budget {
    pub(crate) fn new(limits: &Limits) -> Self {
        Self {
            entries: Arc::new(Semaphore::new(limits.queue_entries)),
            bytes: Arc::new(Semaphore::new(limits.queue_bytes)),
            max_bytes: limits.queue_bytes,
        }
    }

    fn permits(&self, bytes: usize) -> Result<u32, Oversized> {
        u32::try_from(bytes)
            .ok()
            .filter(|_| bytes <= self.max_bytes)
            .ok_or(Oversized)
    }

    /// Admits an item now, or `None` while the queue is full.
    pub(crate) fn try_admit(&self, bytes: usize) -> Option<Held> {
        let bytes = self.permits(bytes).ok()?;
        let entry = Arc::clone(&self.entries).try_acquire_owned().ok()?;
        let bytes = Arc::clone(&self.bytes).try_acquire_many_owned(bytes).ok()?;
        Some(Held {
            _entry: entry,
            _bytes: bytes,
        })
    }

    /// Admits an item once there is room.
    pub(crate) async fn admit(&self, bytes: usize) -> Result<Held, Oversized> {
        let bytes = self.permits(bytes)?;
        // Never closed, so acquiring only waits.
        let entry = Arc::clone(&self.entries)
            .acquire_owned()
            .await
            .map_err(|_| Oversized)?;
        let bytes = Arc::clone(&self.bytes)
            .acquire_many_owned(bytes)
            .await
            .map_err(|_| Oversized)?;
        Ok(Held {
            _entry: entry,
            _bytes: bytes,
        })
    }
}

/// What has happened to the process and its stdin.
#[derive(Clone, Debug, Default)]
pub(crate) struct Status {
    pub(crate) stop_requested: bool,
    /// Set once the supervisor has reaped the process.
    pub(crate) exited: Option<Result<ExitStatus, String>>,
    /// The first failed stdin write.
    pub(crate) write_failure: Option<String>,
}

/// Terminates the process group now, past every queue and the runtime.
#[derive(Clone, Debug)]
pub(crate) struct Stop {
    supervisor: std::sync::mpsc::Sender<()>,
    status: Arc<watch::Sender<Status>>,
}

impl Stop {
    pub(crate) fn stop(&self) {
        self.status
            .send_if_modified(|status| !std::mem::replace(&mut status.stop_requested, true));
        let _ = self.supervisor.send(());
    }
}

/// One stdout line without its line ending. It counts against the stdout
/// queue until dropped.
#[derive(Debug)]
pub(crate) struct Line {
    pub(crate) text: String,
    _held: Held,
}

/// Why stdout yields no more lines.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum End {
    Eof,
    /// The process exited and draining what it left stopped before EOF.
    Drained,
    /// A read failure, invalid UTF-8 or an overlong line.
    Failed(String),
}

/// The process's stdout, in order.
pub(crate) struct Incoming {
    lines: mpsc::UnboundedReceiver<Line>,
    end: Arc<Mutex<Option<End>>>,
    status: watch::Receiver<Status>,
    drain_until: Option<Instant>,
    ended: Option<End>,
}

impl Incoming {
    /// The next line, or `None` once stdout has ended (see `end`). After the
    /// process exits, lines it left keep coming until EOF, until none
    /// completes for 100 ms, or until two seconds have passed, so a
    /// descendant holding stdout open cannot hold the connection open.
    /// Cancellation safe.
    pub(crate) async fn next(&mut self) -> Option<Line> {
        if self.ended.is_some() {
            return None;
        }
        loop {
            if self.drain_until.is_none() && self.status.borrow_and_update().exited.is_some() {
                self.drain_until = Some(Instant::now() + DRAIN_CAP);
            }
            let quiet = self
                .drain_until
                .map(|cap| cap.min(Instant::now() + DRAIN_QUIET));
            tokio::select! {
                line = self.lines.recv() => {
                    if line.is_none() {
                        let end = self.end.lock().unwrap_or_else(PoisonError::into_inner).take();
                        self.ended = Some(end.unwrap_or(End::Eof));
                    }
                    return line;
                }
                changed = self.status.changed(), if self.drain_until.is_none() => {
                    if changed.is_err() {
                        self.drain_until = Some(Instant::now() + DRAIN_CAP);
                    }
                }
                () = sleep_until(quiet), if quiet.is_some() => {
                    self.ended = Some(End::Drained);
                    return None;
                }
            }
        }
    }

    pub(crate) fn end(&self) -> Option<&End> {
        self.ended.as_ref()
    }
}

/// Sleeps until `at`; for a disabled `select!` branch when `None`.
pub(crate) async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Admits stdin lines for the writer task, which writes them in order.
#[derive(Clone, Debug)]
pub(crate) struct Outgoing {
    writes: mpsc::UnboundedSender<Write>,
    budget: Budget,
    frame_bytes: usize,
}

#[derive(Debug)]
struct Write {
    line: String,
    done: oneshot::Sender<io::Result<()>>,
    _held: Held,
}

/// Why a line was not admitted. Nothing of it was written.
#[derive(Debug, PartialEq)]
pub(crate) enum Refused {
    /// Longer than one frame.
    TooLarge,
    /// The stdin queue is full.
    Full,
    /// The writer stopped after a write failed.
    Closed,
}

impl Outgoing {
    /// Admits `line` (without its newline) behind earlier lines. The receiver
    /// reports once the whole line has been written and flushed, or why not.
    pub(crate) fn send(&self, line: String) -> Result<oneshot::Receiver<io::Result<()>>, Refused> {
        if line.len() > self.frame_bytes {
            return Err(Refused::TooLarge);
        }
        let held = self.budget.try_admit(line.len() + 1).ok_or(Refused::Full)?;
        let (done, written) = oneshot::channel();
        self.writes
            .send(Write {
                line,
                done,
                _held: held,
            })
            .map_err(|_| Refused::Closed)?;
        Ok(written)
    }
}

/// What the protocol future gets.
pub(crate) struct Endpoints {
    pub(crate) stdout: Incoming,
    pub(crate) stdin: Outgoing,
    pub(crate) status: watch::Receiver<Status>,
    pub(crate) stop: Stop,
}

/// The last stderr records, kept for diagnostics.
#[derive(Debug)]
struct StderrTail {
    lines: VecDeque<String>,
    bytes: usize,
    max_bytes: usize,
    max_lines: usize,
}

impl StderrTail {
    fn push(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > self.max_lines || self.bytes > self.max_bytes {
            let Some(oldest) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= oldest.len();
        }
    }
}

/// The caller's handle on a launched process. Dropping it stops the process
/// if it still runs and waits for the worker and the supervisor.
pub(crate) struct Connection {
    stop: Stop,
    status: watch::Receiver<Status>,
    stderr: Arc<Mutex<StderrTail>>,
    worker: Option<JoinHandle<()>>,
    supervisor: Option<JoinHandle<()>>,
}

impl Connection {
    pub(crate) fn stop_handle(&self) -> Stop {
        self.stop.clone()
    }

    /// The retained stderr records, oldest first, one per line.
    pub(crate) fn stderr(&self) -> String {
        let tail = self.stderr.lock().unwrap_or_else(PoisonError::into_inner);
        tail.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }

    #[cfg(test)]
    fn status(&self) -> Status {
        self.status.borrow().clone()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop.stop();
        let _ = self.status.has_changed();
        for handle in [self.worker.take(), self.supervisor.take()]
            .into_iter()
            .flatten()
        {
            let _ = handle.join();
        }
    }
}

/// Starts `command` in its own process group with piped stdio, and runs
/// `protocol` on a new worker thread until it returns. Then the process is
/// stopped if it still runs, and the runtime shut down within a bound. A
/// failure before `protocol` starts stops and reaps the process first.
pub(crate) fn launch<P, F>(
    mut command: Command,
    limits: Limits,
    protocol: P,
) -> io::Result<Connection>
where
    P: FnOnce(Endpoints) -> F + Send + 'static,
    F: Future<Output = ()> + 'static,
{
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(&mut command);
    let mut child = spawn_process_group(command)?;
    let abandon = |child: &mut Child, error: io::Error| {
        terminate_process_group(child.id(), Some(child));
        let _ = child.wait();
        error
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(abandon(
            &mut child,
            io::Error::other("the process did not expose its stdio"),
        ));
    };
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return Err(abandon(&mut child, error)),
    };
    let (status_sender, status) = watch::channel(Status::default());
    let status_sender = Arc::new(status_sender);
    let (stop_sender, stops) = std::sync::mpsc::channel();
    let stop = Stop {
        supervisor: stop_sender,
        status: Arc::clone(&status_sender),
    };
    // Handed over once the supervisor runs, so a failed start still holds it.
    let (hand_over, handed) = std::sync::mpsc::channel::<Child>();
    let supervisor = {
        let status = Arc::clone(&status_sender);
        thread::Builder::new()
            .name("spark-stdio-supervisor".to_string())
            .spawn(move || {
                if let Ok(child) = handed.recv() {
                    supervise(child, &stops, &status);
                }
            })
    };
    let supervisor = match supervisor {
        Ok(supervisor) => supervisor,
        Err(error) => return Err(abandon(&mut child, error)),
    };
    if let Err(std::sync::mpsc::SendError(mut child)) = hand_over.send(child) {
        return Err(abandon(
            &mut child,
            io::Error::other("the process supervisor did not start"),
        ));
    }
    let stderr_tail = Arc::new(Mutex::new(StderrTail {
        lines: VecDeque::new(),
        bytes: 0,
        max_bytes: limits.stderr_bytes,
        max_lines: limits.stderr_lines,
    }));
    let (ready, started) = std::sync::mpsc::channel();
    let worker = {
        let stop = stop.clone();
        let status = status.clone();
        let stderr_tail = Arc::clone(&stderr_tail);
        thread::Builder::new()
            .name("spark-stdio-worker".to_string())
            .spawn(move || {
                let worker_stop = stop.clone();
                let endpoints = runtime.block_on(async {
                    let stdin = ChildStdin::from_std(stdin)?;
                    let stdout = ChildStdout::from_std(stdout)?;
                    let stderr = ChildStderr::from_std(stderr)?;
                    let (writes, queued) = mpsc::unbounded_channel();
                    let (lines, received) = mpsc::unbounded_channel();
                    let end = Arc::new(Mutex::new(None));
                    tokio::spawn(write_lines(stdin, queued, status_sender));
                    tokio::spawn(read_lines(
                        stdout,
                        limits.frame_bytes,
                        Budget::new(&limits),
                        lines,
                        Arc::clone(&end),
                    ));
                    tokio::spawn(read_stderr(stderr, stderr_tail, limits.stderr_bytes));
                    io::Result::Ok(Endpoints {
                        stdout: Incoming {
                            lines: received,
                            end,
                            status: status.clone(),
                            drain_until: None,
                            ended: None,
                        },
                        stdin: Outgoing {
                            writes,
                            budget: Budget::new(&limits),
                            frame_bytes: limits.frame_bytes,
                        },
                        status,
                        stop: worker_stop,
                    })
                });
                let endpoints = match endpoints {
                    Ok(endpoints) => endpoints,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));
                runtime.block_on(async move { protocol(endpoints).await });
                stop.stop();
                // Bounded: an abandoned blocking pipe operation cannot hold it.
                runtime.shutdown_timeout(RUNTIME_SHUTDOWN);
            })
    };
    let mut connection = Connection {
        stop,
        status,
        stderr: stderr_tail,
        worker: None,
        supervisor: Some(supervisor),
    };
    // Dropping the connection on failure stops, reaps and joins.
    connection.worker = Some(worker?);
    match started.recv() {
        Ok(Ok(())) => Ok(connection),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(io::Error::other("the process worker did not start")),
    }
}

/// Owns the child until it has exited and been reaped: polls it every 20 ms,
/// and terminates its group on a stop, or once every stop handle is gone.
fn supervise(
    mut child: Child,
    stops: &std::sync::mpsc::Receiver<()>,
    status: &watch::Sender<Status>,
) {
    let exited = loop {
        let stop = !matches!(
            stops.recv_timeout(SUPERVISOR_POLL),
            Err(RecvTimeoutError::Timeout)
        );
        match child.try_wait() {
            Ok(Some(exit)) => break Ok(exit),
            Ok(None) if !stop => continue,
            // A stop, or a child it can no longer poll.
            _ => {
                terminate_process_group(child.id(), Some(&mut child));
                break child.wait();
            }
        }
    };
    status.send_modify(|status| status.exited = Some(exited.map_err(|error| error.to_string())));
}

async fn write_lines(
    stdin: ChildStdin,
    mut writes: mpsc::UnboundedReceiver<Write>,
    status: Arc<watch::Sender<Status>>,
) {
    let mut stdin = FramedWrite::new(stdin, LinesCodec::new());
    while let Some(write) = writes.recv().await {
        // Awaited to the end: a partly written line is never started over.
        let failure = match stdin.send(write.line.as_str()).await {
            Ok(()) => {
                let _ = write.done.send(Ok(()));
                continue;
            }
            Err(error) => error.to_string(),
        };
        status.send_modify(|status| {
            status.write_failure.get_or_insert_with(|| failure.clone());
        });
        let _ = write.done.send(Err(io::Error::other(failure.clone())));
        writes.close();
        while let Some(write) = writes.recv().await {
            let _ = write.done.send(Err(io::Error::other(failure.clone())));
        }
        return;
    }
    // Every sender is gone: dropping the writer closes stdin.
}

async fn read_lines(
    stdout: ChildStdout,
    frame_bytes: usize,
    budget: Budget,
    lines: mpsc::UnboundedSender<Line>,
    end: Arc<Mutex<Option<End>>>,
) {
    // One byte over the frame leaves room for the CR of a CRLF ending.
    let mut stdout = FramedRead::new(stdout, LinesCodec::new_with_max_length(frame_bytes + 1));
    let too_long = || End::Failed(format!("stdout line exceeds {frame_bytes} bytes"));
    let reason = loop {
        match stdout.next().await {
            Some(Ok(text)) if text.len() > frame_bytes => break too_long(),
            Some(Ok(text)) => {
                // A full queue stops reading, so the process waits to write.
                let Ok(held) = budget.admit(text.len()).await else {
                    break too_long();
                };
                if lines.send(Line { text, _held: held }).is_err() {
                    return;
                }
            }
            Some(Err(LinesCodecError::MaxLineLengthExceeded)) => break too_long(),
            Some(Err(LinesCodecError::Io(error))) => break End::Failed(error.to_string()),
            None => break End::Eof,
        }
    };
    // Set before the sender drops, so it never needs a slot in the queue.
    *end.lock().unwrap_or_else(PoisonError::into_inner) = Some(reason);
}

/// Keeps reading stderr whatever it holds: an overlong or non-UTF-8 record
/// is skipped, never an error.
async fn read_stderr(mut stderr: ChildStderr, tail: Arc<Mutex<StderrTail>>, max_record: usize) {
    let mut records = LinesCodec::new_with_max_length(max_record);
    let mut buffer = BytesMut::new();
    let keep = |line: String| {
        tail.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line)
    };
    loop {
        buffer.reserve(8 << 10);
        match stderr.read_buf(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        loop {
            match records.decode(&mut buffer) {
                Ok(Some(line)) => keep(line),
                Ok(None) => break,
                Err(_) => {}
            }
        }
    }
    if let Ok(Some(line)) = records.decode_eof(&mut buffer) {
        keep(line);
    }
}

/// Ordered work for one dedicated thread, so callbacks and file writes never
/// run on the runtime. The thread ends once this is dropped and its current
/// job returns; a job that never returns holds only that thread.
pub(crate) struct Blocking {
    jobs: std::sync::mpsc::Sender<(Job, Held)>,
    budget: Budget,
}

type Job = Box<dyn FnOnce() + Send>;

impl Blocking {
    pub(crate) fn spawn(limits: &Limits) -> io::Result<Self> {
        let (jobs, queue) = std::sync::mpsc::channel::<(Job, Held)>();
        thread::Builder::new()
            .name("spark-stdio-blocking".to_string())
            .spawn(move || {
                for (job, _held) in queue {
                    job();
                }
            })?;
        Ok(Self {
            jobs,
            budget: Budget::new(limits),
        })
    }

    /// Queues `job` behind earlier jobs once there is room.
    pub(crate) async fn run(
        &self,
        bytes: usize,
        job: impl FnOnce() + Send + 'static,
    ) -> Result<(), Oversized> {
        let held = self.budget.admit(bytes).await?;
        let _ = self.jobs.send((Box::new(job), held));
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;
    use std::time::Instant as StdInstant;

    const PROMPTLY: Duration = Duration::from_secs(20);

    fn sh(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }

    /// Launches `script` and runs `body` as its protocol, with its result on
    /// the returned receiver.
    fn start<T, Fut>(
        script: &str,
        limits: Limits,
        body: impl FnOnce(Endpoints) -> Fut + Send + 'static,
    ) -> (Connection, std_mpsc::Receiver<T>)
    where
        T: Send + 'static,
        Fut: Future<Output = T> + 'static,
    {
        let (result, received) = std_mpsc::channel();
        let connection = launch(sh(script), limits, move |endpoints| async move {
            let _ = result.send(body(endpoints).await);
        })
        .expect("launch");
        (connection, received)
    }

    async fn read_all(stdout: &mut Incoming) -> Vec<String> {
        let mut lines = Vec::new();
        while let Some(line) = stdout.next().await {
            lines.push(line.text);
        }
        lines
    }

    #[test]
    fn budget_counts_entries_and_bytes_until_released() {
        let budget = Budget::new(&Limits {
            queue_entries: 2,
            queue_bytes: 10,
            ..LIMITS
        });
        let first = budget.try_admit(6).expect("room");
        assert!(budget.try_admit(5).is_none(), "bytes");
        let second = budget.try_admit(4).expect("room");
        assert!(budget.try_admit(0).is_none(), "entries");
        assert!(budget.try_admit(11).is_none(), "larger than the queue");
        drop(first);
        let third = budget.try_admit(6).expect("released");

        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            assert!(budget.admit(11).await.is_err());
            let waiting = budget.admit(1);
            tokio::pin!(waiting);
            assert!(
                tokio::time::timeout(Duration::from_millis(50), &mut waiting)
                    .await
                    .is_err(),
                "full"
            );
            drop(second);
            waiting.await.expect("admitted once released");
        });
        drop(third);
    }

    #[test]
    fn stdout_frames_strip_crlf_and_keep_partial_and_final_lines() {
        let (_connection, result) = start(
            "printf 'one\\r\\ntw'; sleep 0.3; printf 'o\\nlast'",
            LIMITS,
            |mut endpoints| async move {
                let lines = read_all(&mut endpoints.stdout).await;
                (lines, endpoints.stdout.end().cloned())
            },
        );
        let (lines, end) = result.recv_timeout(PROMPTLY).expect("lines");
        assert_eq!(lines, ["one", "two", "last"]);
        assert_eq!(end, Some(End::Eof));
    }

    #[test]
    fn stdout_frame_limit_is_inclusive_and_an_overlong_line_fails() {
        let (_connection, result) = start(
            "head -c 16777216 /dev/zero | tr '\\0' a; printf '\\r\\n'; \
             head -c 16777217 /dev/zero | tr '\\0' b; echo; echo after",
            LIMITS,
            |mut endpoints| async move {
                let lines = read_all(&mut endpoints.stdout).await;
                let lengths = lines.iter().map(String::len).collect::<Vec<_>>();
                (lengths, endpoints.stdout.end().cloned())
            },
        );
        let (lengths, end) = result.recv_timeout(PROMPTLY).expect("lines");
        assert_eq!(lengths, [16 << 20]);
        assert_eq!(
            end,
            Some(End::Failed(
                "stdout line exceeds 16777216 bytes".to_string()
            ))
        );
    }

    #[test]
    fn stdout_invalid_utf8_fails_the_connection() {
        let (_connection, result) = start(
            "printf 'ok\\n\\377\\n'; echo after",
            LIMITS,
            |mut endpoints| async move {
                let lines = read_all(&mut endpoints.stdout).await;
                (lines, endpoints.stdout.end().cloned())
            },
        );
        let (lines, end) = result.recv_timeout(PROMPTLY).expect("lines");
        assert_eq!(lines, ["ok"]);
        assert!(
            matches!(end, Some(End::Failed(ref error)) if error.contains("UTF8")),
            "{end:?}"
        );
    }

    #[test]
    fn stdin_admission_refuses_oversized_and_full_and_counts_lines_in_flight() {
        let limits = Limits {
            frame_bytes: 1 << 20,
            queue_entries: 2,
            queue_bytes: 4 << 20,
            ..LIMITS
        };
        let (queued, is_queued) = std_mpsc::channel();
        // Never reads stdin, so the first line stays partly written.
        let (connection, result) = start("sleep 30", limits, move |mut endpoints| async move {
            let stdin = endpoints.stdin;
            let refused = stdin.send("x".repeat((1 << 20) + 1)).unwrap_err();
            let in_flight = stdin.send("a".repeat(1 << 20)).expect("first");
            let behind = stdin.send("b".repeat(1 << 20)).expect("second");
            let full = stdin.send("c".to_string()).unwrap_err();
            queued.send(()).unwrap();
            let first = in_flight.await;
            let second = behind.await;
            let status = endpoints
                .status
                .wait_for(|status| status.exited.is_some())
                .await;
            assert!(status.is_ok());
            (refused, full, first, second)
        });
        is_queued.recv_timeout(PROMPTLY).expect("queued");
        let stopped = StdInstant::now();
        connection.stop_handle().stop();
        let (refused, full, first, second) = result.recv_timeout(PROMPTLY).expect("settled");
        assert!(stopped.elapsed() < Duration::from_secs(5));
        assert_eq!(refused, Refused::TooLarge);
        assert_eq!(full, Refused::Full);
        // Never reported written, and never restarted.
        assert!(!matches!(first, Ok(Ok(()))), "{first:?}");
        assert!(!matches!(second, Ok(Ok(()))), "{second:?}");
    }

    #[test]
    fn writer_keeps_large_lines_whole_and_in_order_across_wakes() {
        let sent = [
            "x".repeat(1 << 20),
            "small".to_string(),
            "y".repeat(300 << 10),
        ];
        let expected = sent.to_vec();
        let (_connection, result) = start("cat", LIMITS, move |mut endpoints| async move {
            let written = sent
                .into_iter()
                .map(|line| endpoints.stdin.send(line).expect("admitted"))
                .collect::<Vec<_>>();
            let mut ticks = tokio::time::interval(Duration::from_millis(1));
            let mut echoed = Vec::new();
            while echoed.len() < 3 {
                tokio::select! {
                    line = endpoints.stdout.next() => echoed.push(line.expect("echo").text),
                    _ = ticks.tick() => {}
                }
            }
            for written in written {
                written.await.unwrap().expect("written");
            }
            echoed
        });
        assert!(result.recv_timeout(PROMPTLY).expect("echoed") == expected);
    }

    #[test]
    fn broken_stdin_reports_a_write_failure_while_stdout_stays_open() {
        let (_connection, result) = start(
            "exec 0<&-; echo closed; sleep 1; echo still-here",
            LIMITS,
            |mut endpoints| async move {
                let first = endpoints.stdout.next().await.expect("closed").text;
                let written = endpoints
                    .stdin
                    .send("hello".to_string())
                    .expect("admitted")
                    .await
                    .unwrap();
                let failure = endpoints.status.borrow().write_failure.clone();
                let after = endpoints.stdin.send("again".to_string());
                let next = endpoints.stdout.next().await.map(|line| line.text);
                (first, written.is_err(), failure, after.is_err(), next)
            },
        );
        let (first, failed, failure, refused, next) =
            result.recv_timeout(PROMPTLY).expect("result");
        assert_eq!(first, "closed");
        assert!(failed);
        assert!(failure.is_some());
        assert!(refused, "the writer takes nothing after a failure");
        assert_eq!(next.as_deref(), Some("still-here"));
    }

    #[test]
    fn launch_failure_reports_without_a_running_process() {
        let error = launch(
            Command::new("/nonexistent/spark-stdio"),
            LIMITS,
            |_| async {},
        )
        .err()
        .expect("launch fails");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn supervisor_reports_exit_and_stops_a_live_process() {
        let (connection, result) = start("exit 3", LIMITS, |mut endpoints| async move {
            let status = endpoints
                .status
                .wait_for(|status| status.exited.is_some())
                .await
                .unwrap()
                .clone();
            status.exited.unwrap().unwrap().code()
        });
        assert_eq!(result.recv_timeout(PROMPTLY).expect("exit"), Some(3));
        drop(connection);

        let (connection, result) = start("sleep 30", LIMITS, |mut endpoints| async move {
            endpoints
                .status
                .wait_for(|status| status.exited.is_some())
                .await
                .unwrap()
                .stop_requested
        });
        let stopped = StdInstant::now();
        connection.stop_handle().stop();
        assert!(result.recv_timeout(PROMPTLY).expect("stopped"));
        assert!(stopped.elapsed() < Duration::from_secs(5));
        assert!(!connection.status().exited.unwrap().unwrap().success());
    }

    #[test]
    fn drain_after_exit_ends_despite_descendants_holding_stdout() {
        // Silent: the drain ends 100 ms after the last line.
        let (_connection, result) =
            start("sleep 5 & echo hi", LIMITS, |mut endpoints| async move {
                let started = Instant::now();
                let lines = read_all(&mut endpoints.stdout).await;
                (lines, endpoints.stdout.end().cloned(), started.elapsed())
            });
        let (lines, end, elapsed) = result.recv_timeout(PROMPTLY).expect("silent");
        assert_eq!(lines, ["hi"]);
        assert_eq!(end, Some(End::Drained));
        assert!(elapsed < Duration::from_millis(1500), "{elapsed:?}");

        // Chatty: steady output never extends the two-second cap.
        let (_connection, result) = start(
            "(while echo x; do sleep 0.01; done) & sleep 0.2; exit 0",
            LIMITS,
            |mut endpoints| async move {
                let started = Instant::now();
                let lines = read_all(&mut endpoints.stdout).await;
                (
                    lines.len(),
                    endpoints.stdout.end().cloned(),
                    started.elapsed(),
                )
            },
        );
        let (count, end, elapsed) = result.recv_timeout(PROMPTLY).expect("chatty");
        assert!(count > 20, "{count}");
        assert_eq!(end, Some(End::Drained));
        assert!(
            elapsed >= Duration::from_millis(1900) && elapsed < Duration::from_secs(4),
            "{elapsed:?}"
        );
    }

    #[test]
    fn stderr_tail_is_bounded_under_sustained_pressure() {
        let (connection, result) = start(
            "i=0; while [ $i -lt 3000 ]; do echo \"diagnostic $i\" >&2; i=$((i+1)); done; \
             head -c 200000 /dev/zero | tr '\\0' e >&2; printf '\\n\\377\\nlast' >&2; echo done",
            LIMITS,
            |mut endpoints| async move { read_all(&mut endpoints.stdout).await },
        );
        assert_eq!(result.recv_timeout(PROMPTLY).expect("stdout"), ["done"]);
        let started = StdInstant::now();
        while !connection.stderr().ends_with("last") {
            assert!(started.elapsed() < PROMPTLY, "stderr drained");
            thread::sleep(Duration::from_millis(10));
        }
        let stderr = connection.stderr();
        let lines = stderr.lines().collect::<Vec<_>>();
        assert!(lines.len() <= LIMITS.stderr_lines);
        assert!(stderr.len() <= LIMITS.stderr_bytes + LIMITS.stderr_lines);
        assert_eq!(lines[lines.len() - 2], "diagnostic 2999");
        assert!(!stderr.contains("eeee"), "an overlong record is skipped");
    }

    #[test]
    fn blocking_work_runs_in_order_off_the_runtime() {
        let blocking = Blocking::spawn(&LIMITS).expect("blocking thread");
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        let (done, finished) = std_mpsc::channel();
        let runtime_thread = thread::current().id();
        runtime.block_on(async {
            for index in 0..3 {
                let done = done.clone();
                blocking
                    .run(0, move || {
                        done.send((index, thread::current().id())).unwrap();
                    })
                    .await
                    .unwrap();
            }
        });
        let ran = (0..3)
            .map(|_| finished.recv_timeout(PROMPTLY).expect("ran"))
            .collect::<Vec<_>>();
        assert_eq!(
            ran.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(ran.iter().all(|(_, thread)| *thread != runtime_thread));
    }
}

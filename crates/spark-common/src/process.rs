use std::io::{BufRead, BufReader, Read};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::{Result, SparkCommonError};

pub struct ProcessLineReader {
    rx: Receiver<Option<String>>,
    thread: Option<JoinHandle<()>>,
    eof_seen: bool,
}

impl ProcessLineReader {
    pub fn new<R>(stream: R) -> Self
    where
        R: Read + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        if line.ends_with('\n') {
                            line.pop();
                        }
                        if tx.send(Some(line)).is_err() {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(None);
        });

        Self {
            rx,
            thread: Some(thread),
            eof_seen: false,
        }
    }

    pub fn read_line(&mut self, wait: Duration) -> Option<String> {
        if self.eof_seen {
            return None;
        }

        match self.rx.recv_timeout(wait) {
            Ok(Some(line)) => Some(line),
            Ok(None) | Err(RecvTimeoutError::Disconnected) => {
                self.eof_seen = true;
                None
            }
            Err(RecvTimeoutError::Timeout) => None,
        }
    }

    pub fn join(&mut self, timeout: Option<Duration>) -> Result<bool> {
        let Some(thread) = self.thread.take() else {
            return Ok(true);
        };

        if let Some(timeout) = timeout {
            let deadline = Instant::now() + timeout;
            while !thread.is_finished() {
                if Instant::now() >= deadline {
                    self.thread = Some(thread);
                    return Ok(false);
                }
                thread::sleep(Duration::from_millis(1));
            }
        }

        thread
            .join()
            .map_err(|_| SparkCommonError::ProcessReaderJoin)?;
        Ok(true)
    }
}

impl Drop for ProcessLineReader {
    fn drop(&mut self) {
        if self
            .thread
            .as_ref()
            .map(|thread| thread.is_finished())
            .unwrap_or(false)
        {
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

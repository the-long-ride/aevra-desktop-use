//! One window per user. The first instance holds an exclusive lock on
//! `ui.lock` in the config directory and listens on a loopback port, recorded
//! in `ui.port`. A second launch finds the lock taken, tells the first to show
//! its window, and exits.
//!
//! The lock is a file lock rather than a named mutex or a unix `flock` call so
//! one implementation serves every platform, and the OS releases it if the
//! process dies. The wake-up channel is a loopback TCP socket for the same
//! reason; any local process may send `show`, which at worst raises a window.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

const LOCK_FILE: &str = "ui.lock";
const PORT_FILE: &str = "ui.port";
const SHOW: &str = "show";

pub enum Acquired {
    /// This process is the UI. Keep the value alive for as long as it should be.
    First(Instance),
    /// Another process is the UI.
    AlreadyRunning,
}

/// The lock, and the listener that hears `show` from later launches.
pub struct Instance {
    _lock: File,
    stop: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
    port: u16,
}

impl Instance {
    /// Calls `on_show` each time a later launch asks this instance to show itself.
    pub fn on_show(&mut self, on_show: impl Fn() + Send + 'static, listener: TcpListener) {
        let stop = Arc::clone(&self.stop);
        self.listener = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                        let mut line = String::new();
                        if BufReader::new(&mut stream).read_line(&mut line).is_ok()
                            && line.trim() == SHOW
                        {
                            on_show();
                            // Confirm the callback ran before the later instance exits.
                            let _ = stream.write_all(b"ok\n");
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        }));
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.listener.take() {
            let _ = handle.join();
        }
    }
}

fn lock_path(dir: &Path) -> PathBuf {
    dir.join(LOCK_FILE)
}

fn port_path(dir: &Path) -> PathBuf {
    dir.join(PORT_FILE)
}

/// Takes the UI lock in `dir`, or reports that another process has it. The
/// returned instance is not listening yet: call [`Instance::on_show`] with the
/// listener from [`bind_listener`] once there is something to show.
pub fn acquire(dir: &Path) -> std::io::Result<Acquired> {
    std::fs::create_dir_all(dir)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(lock_path(dir))?;
    match file.try_lock() {
        Ok(()) => Ok(Acquired::First(Instance {
            _lock: file,
            stop: Arc::new(AtomicBool::new(false)),
            listener: None,
            port: 0,
        })),
        Err(TryLockError::WouldBlock) => Ok(Acquired::AlreadyRunning),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Binds the wake-up socket on loopback and records its port in `dir`.
pub fn bind_listener(dir: &Path) -> std::io::Result<TcpListener> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    std::fs::write(port_path(dir), listener.local_addr()?.port().to_string())?;
    Ok(listener)
}

/// Tells the running instance to show its window. Retries briefly, since the
/// first instance may still be starting its listener.
pub fn signal_existing(dir: &Path) -> std::io::Result<()> {
    let mut last = std::io::Error::other("the running instance did not answer");
    for _ in 0..30 {
        let port = std::fs::read_to_string(port_path(dir))
            .ok()
            .and_then(|text| text.trim().parse::<u16>().ok());
        if let Some(port) = port {
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
                Ok(mut stream) => {
                    stream.write_all(format!("{SHOW}\n").as_bytes())?;
                    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                    let mut response = String::new();
                    BufReader::new(&mut stream).read_line(&mut response)?;
                    if response.trim() == "ok" {
                        return Ok(());
                    }
                    return Err(std::io::Error::other("running instance did not acknowledge show"));
                }
                Err(e) => last = e,
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn first(dir: &Path) -> Instance {
        match acquire(dir).unwrap() {
            Acquired::First(instance) => instance,
            Acquired::AlreadyRunning => panic!("expected to be the first instance"),
        }
    }

    #[test]
    fn the_second_launch_finds_the_lock_taken_until_the_first_exits() {
        let dir = tempfile::tempdir().unwrap();
        let instance = first(dir.path());
        assert!(matches!(
            acquire(dir.path()).unwrap(),
            Acquired::AlreadyRunning
        ));
        drop(instance);
        assert!(matches!(acquire(dir.path()).unwrap(), Acquired::First(_)));
    }

    #[test]
    fn a_later_launch_wakes_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut instance = first(dir.path());
        let shown = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&shown);
        let listener = bind_listener(dir.path()).unwrap();
        instance.port = listener.local_addr().unwrap().port();
        instance.on_show(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
            listener,
        );

        assert!(matches!(
            acquire(dir.path()).unwrap(),
            Acquired::AlreadyRunning
        ));
        signal_existing(dir.path()).unwrap();
        signal_existing(dir.path()).unwrap();
        // Each signal now waits for an acknowledgement after the callback.
        assert_eq!(shown.load(Ordering::SeqCst), 2);
        drop(instance);
    }

    #[test]
    fn other_messages_do_not_show_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut instance = first(dir.path());
        let shown = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&shown);
        let listener = bind_listener(dir.path()).unwrap();
        let port = listener.local_addr().unwrap().port();
        instance.on_show(
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
            listener,
        );
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream.write_all(b"quit\n").unwrap();
        drop(stream);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(shown.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn signalling_with_no_running_instance_fails_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        // A stale port file pointing at nothing listening.
        let free = {
            let probe = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            probe.local_addr().unwrap().port()
        };
        std::fs::write(port_path(dir.path()), free.to_string()).unwrap();
        assert!(signal_existing(dir.path()).is_err());
    }
}

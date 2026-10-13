//! The backend actor.
//!
//! `WindowsBackend` initialises COM as a single-threaded apartment and holds
//! `RefCell`s, so it is `!Send` and `!Sync`. The actor therefore builds the
//! backend **on its own thread** and serves calls to it FIFO, one at a time.
//! [`BackendHandle`] is the `Send + Sync + Clone` front for that thread.

use crate::backend::{
    ActRequest, BackgroundActRequest, BackgroundActResult, Capabilities, CaptureRequest,
    CaptureResult, DescribeBackgroundRequest, DescribeBackgroundResult, DescribeRequest,
    DescribeResult, DesktopBackend, ReleaseBackgroundSnapshotRequest, ScreenState,
    TargetIdentityRequest, TargetIdentityResult, WindowIdentity,
};
use crate::error::DesktopError;
use serde_json::Value;
use std::future::Future;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::oneshot;

/// One request to the backend.
#[derive(Debug, Clone)]
pub enum Call {
    Connect,
    Windows,
    FocusedWindow,
    ScreenState,
    Describe(DescribeRequest),
    Act(ActRequest),
    Capture(CaptureRequest),
    TargetIdentity(TargetIdentityRequest),
    DescribeBackground(DescribeBackgroundRequest),
    ReleaseBackgroundSnapshot(ReleaseBackgroundSnapshotRequest),
    BackgroundAct(BackgroundActRequest),
}

/// The backend's answer, with the helper's own types kept intact.
#[derive(Debug, Clone)]
pub enum Reply {
    Capabilities(Capabilities),
    Windows(Vec<WindowIdentity>),
    Window(WindowIdentity),
    ScreenState(ScreenState),
    Describe(DescribeResult),
    Bool(bool),
    Capture(CaptureResult),
    TargetIdentity(TargetIdentityResult),
    DescribeBackground(DescribeBackgroundResult),
    BackgroundAct(BackgroundActResult),
}

impl Reply {
    /// The wire shape the helper's stdio protocol would have produced.
    pub fn to_value(&self) -> Value {
        let result = match self {
            Reply::Capabilities(v) => serde_json::to_value(v),
            Reply::Windows(v) => serde_json::to_value(v),
            Reply::Window(v) => serde_json::to_value(v),
            Reply::ScreenState(v) => serde_json::to_value(v),
            Reply::Describe(v) => serde_json::to_value(v),
            Reply::Bool(v) => serde_json::to_value(v),
            Reply::Capture(v) => serde_json::to_value(v),
            Reply::TargetIdentity(v) => serde_json::to_value(v),
            Reply::DescribeBackground(v) => serde_json::to_value(v),
            Reply::BackgroundAct(v) => serde_json::to_value(v),
        };
        result.unwrap_or(Value::Null)
    }
}

fn dispatch(backend: &dyn DesktopBackend, call: Call) -> Result<Reply, String> {
    match call {
        Call::Connect => backend.connect().map(Reply::Capabilities),
        Call::Windows => backend.windows().map(Reply::Windows),
        Call::FocusedWindow => backend.focused_window().map(Reply::Window),
        Call::ScreenState => backend.screen_state().map(Reply::ScreenState),
        Call::Describe(r) => backend.describe(r).map(Reply::Describe),
        Call::Act(r) => backend.act(r).map(Reply::Bool),
        Call::Capture(r) => backend.capture(r).map(Reply::Capture),
        Call::TargetIdentity(r) => backend.target_identity(r).map(Reply::TargetIdentity),
        Call::DescribeBackground(r) => backend
            .describe_background(r)
            .map(Reply::DescribeBackground),
        Call::ReleaseBackgroundSnapshot(r) => {
            backend.release_background_snapshot(r).map(Reply::Bool)
        }
        Call::BackgroundAct(r) => backend.background_act(r).map(Reply::BackgroundAct),
    }
}

type ReplyResult = Result<Reply, DesktopError>;
type ReplyTx = oneshot::Sender<ReplyResult>;
type ReplyRx = oneshot::Receiver<ReplyResult>;

#[allow(clippy::large_enum_variant)]
enum Msg {
    Call(Call, ReplyTx),
    Shutdown,
}

/// Starts one actor thread reading from the receiver and clearing `alive` when
/// it ends. Boxed so the handle can restart the thread after a panic without
/// knowing the backend type.
type Starter = dyn Fn(Receiver<Msg>, Arc<AtomicBool>) + Send + Sync;

struct Worker {
    tx: Sender<Msg>,
    alive: Arc<AtomicBool>,
}

struct Inner {
    start: Box<Starter>,
    worker: Mutex<Option<Worker>>,
    closed: AtomicBool,
}

/// `Send + Sync + Clone` handle to the backend thread.
#[derive(Clone)]
pub struct BackendHandle {
    inner: Arc<Inner>,
}

fn died() -> DesktopError {
    DesktopError::driver_died("the desktop backend stopped before answering")
}

/// A queued call. The call is already in the actor's queue when this value
/// exists, so submitting in order guarantees execution in that order.
pub struct PendingTyped {
    rx: Result<ReplyRx, DesktopError>,
}

impl Future for PendingTyped {
    type Output = ReplyResult;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.rx {
            Err(e) => Poll::Ready(Err(e.clone())),
            Ok(rx) => match Pin::new(rx).poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(Ok(r)) => Poll::Ready(r),
                Poll::Ready(Err(_)) => Poll::Ready(Err(died())),
            },
        }
    }
}

/// Like [`PendingTyped`] but resolves to the wire-shaped JSON value.
pub struct PendingCall(PendingTyped);

impl Future for PendingCall {
    type Output = Result<Value, DesktopError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0)
            .poll(cx)
            .map(|r| r.map(|reply| reply.to_value()))
    }
}

/// Clears the worker's `alive` flag when the actor thread ends for any reason,
/// including a panic that escapes.
struct AliveGuard(Arc<AtomicBool>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl BackendHandle {
    /// Spawns the actor thread. `make` runs **on that thread**, so the backend
    /// it builds need not be `Send`. `make` is `Fn` rather than `FnOnce`
    /// because the thread is rebuilt after a backend panic.
    pub fn spawn<F, B>(make: F) -> BackendHandle
    where
        F: Fn() -> B + Send + Sync + 'static,
        B: DesktopBackend + 'static,
    {
        let make = Arc::new(make);
        let start: Box<Starter> = Box::new(move |rx, alive| {
            let make = Arc::clone(&make);
            std::thread::Builder::new()
                .name("desktop-backend".into())
                .spawn(move || {
                    let guard = AliveGuard(alive);
                    run_actor(make.as_ref(), rx, &guard.0);
                })
                .expect("failed to spawn the desktop backend thread");
        });
        BackendHandle {
            inner: Arc::new(Inner {
                start,
                worker: Mutex::new(None),
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// Queues a call and returns immediately. Calls run FIFO in submission order.
    pub fn submit(&self, call: Call) -> PendingCall {
        PendingCall(self.submit_typed(call))
    }

    /// Queues a call, keeping the helper's typed reply.
    pub fn submit_typed(&self, call: Call) -> PendingTyped {
        PendingTyped {
            rx: self.enqueue(call),
        }
    }

    /// Runs one call and returns the helper's wire-shaped JSON result.
    pub async fn call(&self, call: Call) -> Result<Value, DesktopError> {
        self.submit(call).await
    }

    /// Runs one call and returns the typed reply.
    pub async fn call_typed(&self, call: Call) -> Result<Reply, DesktopError> {
        self.submit_typed(call).await
    }

    /// Stops the actor thread. Later calls fail with `DESKTOP_DRIVER_DIED`.
    pub fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::SeqCst);
        if let Some(worker) = self.inner.worker.lock().unwrap().take() {
            let _ = worker.tx.send(Msg::Shutdown);
        }
    }

    fn enqueue(&self, call: Call) -> Result<ReplyRx, DesktopError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(DesktopError::driver_died(
                "the desktop backend was shut down",
            ));
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut msg = Msg::Call(call, reply_tx);
        let mut guard = self.inner.worker.lock().unwrap();
        // At most two attempts: the current worker, then a freshly started one.
        for _ in 0..2 {
            let worker = guard.get_or_insert_with(|| self.start_worker());
            if !worker.alive.load(Ordering::SeqCst) {
                *guard = None;
                continue;
            }
            match worker.tx.send(msg) {
                Ok(()) => return Ok(reply_rx),
                Err(std::sync::mpsc::SendError(returned)) => {
                    msg = returned;
                    *guard = None;
                }
            }
        }
        Err(DesktopError::driver_died(
            "the desktop backend could not be restarted",
        ))
    }

    fn start_worker(&self) -> Worker {
        let (tx, rx) = channel();
        let alive = Arc::new(AtomicBool::new(true));
        (self.inner.start)(rx, Arc::clone(&alive));
        Worker { tx, alive }
    }
}

fn run_actor<F, B>(make: &F, rx: Receiver<Msg>, alive: &AtomicBool)
where
    F: Fn() -> B,
    B: DesktopBackend,
{
    let backend = match catch_unwind(AssertUnwindSafe(make)) {
        Ok(backend) => backend,
        Err(_) => {
            alive.store(false, Ordering::SeqCst);
            // The backend could not be built. Answer anything already queued;
            // the handle restarts the thread on the next call.
            while let Ok(Msg::Call(_, reply)) = rx.try_recv() {
                let _ = reply.send(Err(DesktopError::driver_died(
                    "the desktop backend failed to start",
                )));
            }
            return;
        }
    };
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Shutdown => break,
            Msg::Call(call, reply) => {
                match catch_unwind(AssertUnwindSafe(|| dispatch(&backend, call))) {
                    Ok(result) => {
                        let _ = reply.send(result.map_err(DesktopError::from_helper));
                    }
                    Err(_) => {
                        // Mark the worker dead before answering, so a caller that
                        // reacts to the failure with a new call gets a new thread.
                        alive.store(false, Ordering::SeqCst);
                        let _ = reply.send(Err(DesktopError::driver_died(
                            "the desktop backend panicked",
                        )));
                        // The backend's state is unknown after a panic. End the
                        // thread; the handle starts a new one on the next call.
                        break;
                    }
                }
            }
        }
    }
}

/// The native backend for this platform, running on its own actor thread.
pub fn native() -> BackendHandle {
    BackendHandle::spawn(crate::new_native_backend)
}

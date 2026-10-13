//! The canonical helper library behind the standalone backend actor.
//! Native source, platform cfg and dependencies are owned by the helper crate.
pub use aevra_desktop_helper::{backend, new_native_backend, target_guard};
pub use backend::{
    require_authorized_target, ActRequest, BackgroundActRequest, BackgroundActResult, Capabilities,
    CaptureRequest, CaptureResult, DescribeBackgroundRequest, DescribeBackgroundResult,
    DescribeNode, DescribeRequest, DescribeResult, DesktopBackend,
    ReleaseBackgroundSnapshotRequest, ScreenState, TargetIdentityRequest, TargetIdentityResult,
    VerifiedWindowHost, WindowIdentity,
};
pub use target_guard::WindowInstance;
pub mod actor;
pub mod error;
pub use actor::{BackendHandle, Call, Reply};
pub use error::DesktopError;

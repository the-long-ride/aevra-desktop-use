//! Validate read source and current policy before data or refs cross the host boundary.
use super::*;
use desktop_core::{BackendHandle, WindowIdentity};
impl Server {
    pub(super) async fn verify_read_result(
        &self,
        backend: &BackendHandle,
        expected: &desktop_core::TargetIdentityResult,
        observed: Option<&WindowIdentity>,
        entry: &mut ActivityEntry,
    ) -> Result<(), DesktopError> {
        let actual = self
            .target_identity(backend, &expected.window.window_id)
            .await?;
        desktop_core::require_authorized_target(Some(expected), &actual)
            .map_err(DesktopError::from_helper)?;
        let observed = observed.ok_or_else(|| {
            DesktopError::new(
                "DESKTOP_TARGET_CHANGED",
                "Read returned no window attribution",
            )
        })?;
        if observed.window_id != expected.window.window_id
            || observed.executable_path != expected.window.executable_path
            || observed.process_name != expected.window.process_name
        {
            return Err(DesktopError::new(
                "DESKTOP_TARGET_CHANGED",
                "Read source differs from the authorized window",
            ));
        }
        self.authorize(&actual, entry)?;
        Ok(())
    }
}

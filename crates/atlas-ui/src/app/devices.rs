//! Removable-disk (udisks2) action handlers. Mount/unmount shell out to
//! `udisksctl` on a blocking worker so the UI never stalls; results return via
//! `BgResult` and are drained on the UI thread.

use crate::app::App;
use crate::state::app_state::BgResult;

impl App {
    pub(crate) fn mount_device(&mut self, dev: String) {
        let tx = self.tx.clone();
        self.rt.spawn_blocking(move || {
            let msg = match crate::fs::udisks::mount(&dev) {
                Ok(path) => BgResult::DeviceMounted(path),
                Err(e) => BgResult::DeviceActionFailed(e.to_string()),
            };
            let _ = tx.send(msg);
        });
    }

    pub(crate) fn unmount_device(&mut self, dev: String) {
        let tx = self.tx.clone();
        self.rt.spawn_blocking(move || {
            let msg = match crate::fs::udisks::unmount(&dev) {
                Ok(()) => BgResult::DevicesChanged,
                Err(e) => BgResult::DeviceActionFailed(e.to_string()),
            };
            let _ = tx.send(msg);
        });
    }
}

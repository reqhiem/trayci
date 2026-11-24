//! The Windows tray icon. W4 builds it on `tray-icon`; until then Windows has no tray.
use crate::popover::Popover;
use tokio::{runtime::Handle, sync::watch};

pub fn spawn(_runtime: &Handle, _popover: Popover, _tooltips: watch::Receiver<String>) {}

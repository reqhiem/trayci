//! StatusNotifierItem via ksni, no GTK: unlike libappindicator it gets left clicks and where (#97).
use crate::popover::Popover;
use ksni::{menu::StandardItem, Icon, MenuItem, OfflineReason, ToolTip, TrayMethods};
use slint::winit_030::winit::dpi::PhysicalPosition;
use tokio::{runtime::Handle, sync::watch};

/// Every size a panel is likely to draw, so the host scales none of them.
const ICONS: [&[u8]; 4] = [
    include_bytes!("../../resources/icons/16x16.png"),
    include_bytes!("../../resources/icons/32x32.png"),
    include_bytes!("../../resources/icons/48x48.png"),
    include_bytes!("../../resources/icons/64x64.png"),
];

struct Tray {
    popover: Popover,
    icons: Vec<Icon>,
    tooltip: String,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "trayci".into()
    }

    fn title(&self) -> String {
        "Trayci".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.tooltip.clone(),
            ..Default::default()
        }
    }

    /// `x` and `y` are where the click happened, in screen pixels: X11 has no other kind.
    fn activate(&mut self, x: i32, y: i32) {
        let popover = self.popover.clone();
        let anchor = PhysicalPosition::new(x.into(), y.into());
        let _ = slint::invoke_from_event_loop(move || popover.toggle(Some(anchor)));
    }

    /// Keeps serving: ksni registers once a watcher appears, as at login before the panel starts.
    fn watcher_offline(&self, _reason: OfflineReason) -> bool {
        eprintln!(
            "trayci: no StatusNotifierWatcher on the session bus, so no tray icon until one \
             starts; Ctrl+Alt+U or running `trayci tray` again opens the popover"
        );
        true
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Show Trayci".into(),
                activate: Box::new(|tray: &mut Self| {
                    let popover = tray.popover.clone();
                    let _ = slint::invoke_from_event_loop(move || popover.show(None));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_| {
                    let _ = slint::quit_event_loop();
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Registers the tray icon; its tooltip follows `tooltips` while the sender lives.
pub fn spawn(runtime: &Handle, popover: Popover, mut tooltips: watch::Receiver<String>) {
    let icons = match ICONS.into_iter().map(icon).collect() {
        Ok(icons) => icons,
        Err(error) => {
            eprintln!("trayci: unreadable tray icon: {error}");
            return;
        }
    };
    runtime.spawn(async move {
        let tooltip = tooltips.borrow_and_update().clone();
        let tray = Tray {
            popover,
            icons,
            tooltip,
        };
        let handle = match tray.assume_sni_available(true).spawn().await {
            Ok(handle) => handle,
            Err(error) => {
                eprintln!("trayci: no tray icon: {error}");
                return;
            }
        };
        while tooltips.changed().await.is_ok() {
            let tooltip = tooltips.borrow_and_update().clone();
            handle.update(|tray| tray.tooltip = tooltip).await;
        }
    });
}

/// The PNG as StatusNotifierItem wants it: ARGB32 in network byte order.
fn icon(png: &[u8]) -> Result<Icon, png::DecodingError> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png)).read_info()?;
    let mut data = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let frame = reader.next_frame(&mut data)?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err(png::DecodingError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the tray icon must be 8-bit RGBA",
        )));
    }
    data.truncate(frame.buffer_size());
    data.as_chunks_mut::<4>()
        .0
        .iter_mut()
        .for_each(|pixel| pixel.rotate_right(1));
    Ok(Icon {
        width: i32::try_from(frame.width).unwrap_or(0),
        height: i32::try_from(frame.height).unwrap_or(0),
        data,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_icon_decodes_to_argb() {
        for (png, size) in super::ICONS.into_iter().zip([16, 32, 48, 64]) {
            let icon = super::icon(png).expect("bundled icon decodes");
            assert_eq!((icon.width, icon.height), (size, size));
            assert_eq!(icon.data.len(), (size * size * 4) as usize);
        }
    }
}

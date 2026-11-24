//! A StatusNotifierItem through ksni, with no GTK. Unlike libappindicator it is told about a left
//! click, and where it happened (issue #97).
use crate::popover::Popover;
use ksni::{menu::StandardItem, Icon, MenuItem, ToolTip, TrayMethods};
use slint::winit_030::winit::dpi::PhysicalPosition;
use tokio::{runtime::Handle, sync::watch};

const ICON: &[u8] = include_bytes!("../../resources/icons/32x32.png");

struct Tray {
    popover: Popover,
    icon: Icon,
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
        vec![self.icon.clone()]
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: self.tooltip.clone(),
            ..Default::default()
        }
    }

    fn activate(&mut self, x: i32, y: i32) {
        let popover = self.popover.clone();
        let anchor = PhysicalPosition::new(x.into(), y.into());
        let _ = slint::invoke_from_event_loop(move || popover.toggle(Some(anchor)));
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

/// Registers the tray icon and keeps its tooltip on the latest of `tooltips` for as long as the
/// sender lives.
pub fn spawn(runtime: &Handle, popover: Popover, mut tooltips: watch::Receiver<String>) {
    let icon = match icon() {
        Ok(icon) => icon,
        Err(error) => {
            eprintln!("trayci: unreadable tray icon: {error}");
            return;
        }
    };
    runtime.spawn(async move {
        let tooltip = tooltips.borrow_and_update().clone();
        let tray = Tray {
            popover,
            icon,
            tooltip,
        };
        let handle = match tray.spawn().await {
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
fn icon() -> Result<Icon, png::DecodingError> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(ICON)).read_info()?;
    let mut data = vec![0; reader.output_buffer_size().unwrap_or_default()];
    let frame = reader.next_frame(&mut data)?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return Err(png::DecodingError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the tray icon must be 8-bit RGBA",
        )));
    }
    data.truncate(frame.buffer_size());
    data.chunks_exact_mut(4)
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
    fn the_icon_decodes_to_argb() {
        let icon = super::icon().expect("bundled icon decodes");
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.data.len(), 32 * 32 * 4);
    }
}

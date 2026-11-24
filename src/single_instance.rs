//! One tray app per config dir: a second launch asks the first to show its popover, then exits.
//! It is also how to open the popover without the tray: run `trayci tray` again.

#[cfg(target_os = "linux")]
pub use linux::claim;

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        hash::{DefaultHasher, Hash, Hasher},
        io::{Read, Write},
        os::{
            linux::net::SocketAddrExt,
            unix::net::{SocketAddr, UnixListener, UnixStream},
        },
        time::Duration,
    };

    const SHOW: &[u8] = b"show";

    /// The first instance; `None` inside when the socket could not be bound, so it runs alone.
    pub struct Instance(Option<UnixListener>);

    /// `None` when another instance runs for this config dir: it has been asked to show its
    /// popover, and this one should exit.
    pub fn claim() -> Option<Instance> {
        let address = match SocketAddr::from_abstract_name(name()) {
            Ok(address) => address,
            Err(error) => {
                eprintln!("trayci: no single-instance socket: {error}");
                return Some(Instance(None));
            }
        };
        if let Ok(mut first) = UnixStream::connect_addr(&address) {
            let _ = first.write_all(SHOW);
            return None;
        }
        match UnixListener::bind_addr(&address) {
            Ok(listener) => Some(Instance(Some(listener))),
            Err(error) => {
                eprintln!("trayci: no single-instance socket: {error}");
                Some(Instance(None))
            }
        }
    }

    impl Instance {
        /// Calls `show`, on a thread of its own, for every later launch.
        pub fn serve(self, show: impl Fn() + Send + 'static) {
            let Some(listener) = self.0 else {
                return;
            };
            std::thread::spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    // Any user can reach an abstract socket; one that never writes must not
                    // block the launches behind it.
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    let mut request = Vec::new();
                    let read = (&mut stream).take(16).read_to_end(&mut request);
                    if read.is_ok() && request == SHOW {
                        show();
                    }
                }
            });
        }
    }

    /// Abstract socket names are shared by every user on the machine, so the name carries the
    /// config dir: one instance per user, and one per `XDG_CONFIG_HOME` when testing.
    fn name() -> Vec<u8> {
        let name = format!("trayci:{}", trayci_core::settings::config_base().display());
        if name.len() <= 100 {
            return name.into_bytes();
        }
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        format!("trayci:{:016x}", hasher.finish()).into_bytes()
    }
}

/// W4 replaces this with a named pipe or mutex; until then every launch on Windows runs.
#[cfg(windows)]
pub struct Instance;

#[cfg(windows)]
pub fn claim() -> Option<Instance> {
    Some(Instance)
}

#[cfg(windows)]
impl Instance {
    pub fn serve(self, _show: impl Fn() + Send + 'static) {}
}

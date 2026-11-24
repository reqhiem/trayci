//! One tray app per config dir: a second launch asks the first to show its popover, then exits.
//! It is also how to open the popover without the tray: run `trayci tray` again.

#[cfg(target_os = "linux")]
pub use linux::claim;

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        io::{ErrorKind, Read, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::{Path, PathBuf},
        time::Duration,
    };

    const SHOW: &[u8] = b"show";

    /// The first instance; `None` inside when there is no socket, so it runs alone.
    pub struct Instance(Option<UnixListener>);

    /// `None` when another instance runs for this config dir: it has been asked to show its
    /// popover, and this one should exit.
    ///
    /// The socket lives in `XDG_RUNTIME_DIR`, which only its user can enter. An abstract socket
    /// let any local user take the name first, so every later launch connected to theirs and
    /// exited, or send `show` to ours.
    pub fn claim() -> Option<Instance> {
        let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty())
        else {
            eprintln!("trayci: XDG_RUNTIME_DIR is unset, so a second launch starts a second tray");
            return Some(Instance(None));
        };
        let path = socket_path(
            Path::new(&runtime_dir),
            &trayci_core::settings::config_base(),
        );
        match UnixStream::connect(&path) {
            Ok(mut first) => {
                let _ = first.write_all(SHOW);
                return None;
            }
            // Left behind by an instance that did not exit cleanly.
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                let _ = std::fs::remove_file(&path);
            }
            Err(_) => {}
        }
        match UnixListener::bind(&path) {
            Ok(listener) => Some(Instance(Some(listener))),
            // Another launch bound it between our connect and our bind: it is the first.
            Err(error) if error.kind() == ErrorKind::AddrInUse => None,
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
                    // One launch that never writes must not block the launches behind it.
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

    /// One socket per config dir, so a test run with its own `XDG_CONFIG_HOME` is its own
    /// instance. The name hashes the dir to stay under the 108-byte socket path limit, with FNV-1a
    /// because `DefaultHasher` may change between Rust releases and two builds would then miss
    /// each other.
    fn socket_path(runtime_dir: &Path, config_base: &Path) -> PathBuf {
        let hash = config_base
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
            });
        runtime_dir.join(format!("trayci-{hash:016x}.sock"))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_socket_is_per_config_dir_and_stable() {
            let run = Path::new("/run/user/1000");
            assert_eq!(
                socket_path(run, Path::new("")),
                Path::new("/run/user/1000/trayci-cbf29ce484222325.sock"),
                "FNV-1a's offset basis: the name must not change between builds"
            );
            assert_eq!(
                socket_path(run, Path::new("a")),
                Path::new("/run/user/1000/trayci-af63dc4c8601ec8c.sock")
            );
            assert_ne!(
                socket_path(run, Path::new("/home/a/.config/trayci")),
                socket_path(run, Path::new("/tmp/x/trayci"))
            );
        }
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

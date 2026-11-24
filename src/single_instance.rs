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

#[cfg(windows)]
pub use windows::claim;

#[cfg(windows)]
mod windows {
    use std::{
        fs::{File, OpenOptions},
        hash::{DefaultHasher, Hash, Hasher},
        io::{self, Read, Write},
        os::windows::io::{AsRawHandle, FromRawHandle},
        sync::{Arc, Mutex},
    };
    use windows_sys::Win32::{
        Foundation::{
            ERROR_ACCESS_DENIED, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND},
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
            PIPE_UNLIMITED_INSTANCES,
        },
    };

    const SHOW: &[u8] = b"show";

    /// The first instance; `None` inside when the pipe could not be made, so it runs alone.
    pub struct Instance(Option<File>);

    /// `None` when another instance runs for this config dir: it has been asked to show its
    /// popover, and this one should exit.
    pub fn claim() -> Option<Instance> {
        match listen(true) {
            Ok(pipe) => Some(Instance(Some(pipe))),
            // Only a pipe that already exists refuses a first instance.
            Err(error) if error.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                let shown = OpenOptions::new()
                    .write(true)
                    .open(name())
                    .and_then(|mut first| first.write_all(SHOW));
                if let Err(error) = shown {
                    eprintln!("trayci: could not reach the running instance: {error}");
                }
                None
            }
            Err(error) => {
                eprintln!("trayci: no single-instance pipe: {error}");
                Some(Instance(None))
            }
        }
    }

    impl Instance {
        /// Calls `show`, from a thread of its own, for every later launch.
        pub fn serve(self, show: impl Fn() + Send + 'static) {
            let Some(mut pipe) = self.0 else {
                return;
            };
            let show = Arc::new(Mutex::new(show));
            std::thread::spawn(move || loop {
                // SAFETY: a pipe handle this thread owns; with no OVERLAPPED the call blocks until
                // a client connects.
                let connected =
                    unsafe { ConnectNamedPipe(pipe.as_raw_handle(), std::ptr::null_mut()) } != 0;
                // A launch that connected before the call, or that already wrote and closed:
                // what it wrote is still in the pipe.
                let early = [ERROR_PIPE_CONNECTED, ERROR_NO_DATA]
                    .map(|code| Some(code as i32))
                    .contains(&io::Error::last_os_error().raw_os_error());
                let client = match listen(false) {
                    Ok(next) => std::mem::replace(&mut pipe, next),
                    Err(error) => {
                        eprintln!("trayci: single-instance pipe closed: {error}");
                        return;
                    }
                };
                if !connected && !early {
                    continue;
                }
                // Every user may open the pipe for reading; one that never writes must not
                // block the launches behind it.
                let show = Arc::clone(&show);
                std::thread::spawn(move || {
                    let mut request = Vec::new();
                    let read = (&client).take(16).read_to_end(&mut request);
                    if read.is_ok() && request == SHOW {
                        (show.lock().expect("single-instance lock"))();
                    }
                });
            });
        }
    }

    fn listen(first: bool) -> io::Result<File> {
        let name: Vec<u16> = name().encode_utf16().chain([0]).collect();
        let first = if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
        // SAFETY: `name` is NUL-terminated and outlives the call.
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_INBOUND | first,
                PIPE_TYPE_BYTE | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                0,
                64,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a valid handle that nothing else owns.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    /// Pipe names are machine-wide and cannot hold a backslash, so the name is a hash of the
    /// config dir: one instance per user, and one per config dir when testing.
    fn name() -> String {
        let mut hasher = DefaultHasher::new();
        trayci_core::settings::config_base().hash(&mut hasher);
        format!(r"\\.\pipe\trayci-{:016x}", hasher.finish())
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn a_second_launch_asks_the_first_to_show() {
            let first = super::claim().expect("the first launch owns the pipe");
            let (shown, show) = std::sync::mpsc::channel();
            first.serve(move || {
                let _ = shown.send(());
            });
            assert!(super::claim().is_none(), "a second launch exits");
            show.recv_timeout(std::time::Duration::from_secs(5))
                .expect("the first instance was asked to show");
        }
    }
}

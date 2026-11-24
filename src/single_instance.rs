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
        Foundation::{ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE},
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
                    unsafe { ConnectNamedPipe(pipe.as_raw_handle(), std::ptr::null_mut()) != 0 }
                        || io::Error::last_os_error().raw_os_error()
                            == Some(ERROR_PIPE_CONNECTED as i32);
                let client = match listen(false) {
                    Ok(next) => std::mem::replace(&mut pipe, next),
                    Err(error) => {
                        eprintln!("trayci: single-instance pipe closed: {error}");
                        return;
                    }
                };
                if !connected {
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
}

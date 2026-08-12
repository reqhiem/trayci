use crate::model::{ProviderError, ProviderErrorKind};
use chrono::DateTime;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, PtySize};
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::Duration,
};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

static CHILDREN: LazyLock<Mutex<HashMap<u64, Box<dyn ChildKiller + Send + Sync>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_CHILD: AtomicU64 = AtomicU64::new(1);

pub async fn resolve_executable(name: &str, configured: Option<&Path>) -> Option<PathBuf> {
    if configured.is_some_and(executable) {
        return configured.map(Path::to_path_buf);
    }
    let mut candidates: Vec<PathBuf> = env::var_os("PATH")
        .map(|path| env::split_paths(&path).map(|dir| dir.join(name)).collect())
        .unwrap_or_default();
    if let Some(home) = home_dir() {
        candidates.extend([
            home.join(".local/bin").join(name),
            home.join(".npm-global/bin").join(name),
            home.join(".bun/bin").join(name),
        ]);
    }
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .find(|path| seen.insert(path.clone()) && executable(path))
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

pub fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

pub fn bounded_cwd() -> Result<PathBuf, ProviderError> {
    let root = env::var_os("XDG_RUNTIME_DIR").map_or_else(
        || env::temp_dir().join(format!("trayci-{}", user_id())),
        |value| PathBuf::from(value).join("trayci"),
    );
    let directory = root.join("provider-probe");
    fs::create_dir_all(&directory).map_err(|error| {
        ProviderError::new(
            ProviderErrorKind::Unknown,
            format!("probe directory: {error}"),
        )
    })?;
    #[cfg(unix)]
    for path in [&root, &directory] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            ProviderError::new(
                ProviderErrorKind::Unknown,
                format!("probe permissions: {error}"),
            )
        })?;
    }
    Ok(directory)
}

fn user_id() -> String {
    env::var("UID")
        .or_else(|_| env::var("USERNAME"))
        .or_else(|_| env::var("USER"))
        .unwrap_or_else(|_| "user".into())
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect()
}

pub fn abort_all_children() {
    for (_, mut child) in CHILDREN.lock().expect("child registry").drain() {
        let _ = child.kill();
    }
}

pub fn active_child_count() -> usize {
    CHILDREN.lock().expect("child registry").len()
}

pub struct PtyOptions<'a> {
    pub executable: &'a Path,
    pub args: &'a [&'a str],
    pub input: &'a str,
    pub write_delay: Duration,
    pub completion_delay: Duration,
    pub timeout: Duration,
    pub cancellation: CancellationToken,
    pub complete: Arc<dyn Fn(&str) -> bool + Send + Sync>,
}

pub async fn run_pty(options: PtyOptions<'_>) -> Result<String, ProviderError> {
    if options.cancellation.is_cancelled() {
        return Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled"));
    }
    let executable = options.executable.to_path_buf();
    let args = options
        .args
        .iter()
        .map(|arg| (*arg).to_owned())
        .collect::<Vec<_>>();
    let input = options.input.to_owned();
    let cwd = bounded_cwd()?;
    let complete = options.complete;
    let write_delay = options.write_delay;
    let completion_delay = options.completion_delay;
    let (started_tx, started_rx) = oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| {
                ProviderError::new(ProviderErrorKind::Unknown, format!("PTY: {error}"))
            })?;
        let mut command = CommandBuilder::new(executable);
        command.args(args);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command).map_err(|error| {
            ProviderError::new(ProviderErrorKind::Unknown, format!("CLI start: {error}"))
        })?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let id = NEXT_CHILD.fetch_add(1, Ordering::Relaxed);
        CHILDREN.lock().expect("child registry").insert(id, killer);
        let _ = started_tx.send(id);
        let mut reader = pair.master.try_clone_reader().map_err(|error| {
            ProviderError::new(ProviderErrorKind::Unknown, format!("PTY reader: {error}"))
        })?;
        let mut writer = pair.master.take_writer().map_err(|error| {
            ProviderError::new(ProviderErrorKind::Unknown, format!("PTY writer: {error}"))
        })?;
        std::thread::sleep(write_delay);
        writer
            .write_all(format!("{input}\r").as_bytes())
            .map_err(|error| {
                ProviderError::new(ProviderErrorKind::Unknown, format!("PTY input: {error}"))
            })?;
        writer.flush().ok();
        let mut output = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            let read = reader.read(&mut chunk).unwrap_or(0);
            if read == 0 {
                break;
            }
            output.extend_from_slice(&chunk[..read]);
            if output.len() > 1_000_000 {
                output.drain(..output.len() - 1_000_000);
            }
            let clean = strip_terminal_codes(&String::from_utf8_lossy(&output));
            if complete(&clean) {
                std::thread::sleep(completion_delay);
                return Ok(clean);
            }
        }
        Err(ProviderError::new(
            ProviderErrorKind::Parse,
            "CLI exited before usage was available",
        ))
    });
    let id = started_rx
        .await
        .map_err(|_| ProviderError::new(ProviderErrorKind::Unknown, "CLI failed to start"))?;
    let outcome = tokio::select! {
        result = task => result.map_err(|error| ProviderError::new(ProviderErrorKind::Unknown, format!("PTY task: {error}")))?,
        _ = options.cancellation.cancelled() => Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled")),
        _ = tokio::time::sleep(options.timeout) => Err(ProviderError::new(ProviderErrorKind::Timeout, "CLI probe timed out")),
    };
    if let Some(child) = CHILDREN.lock().expect("child registry").get_mut(&id) {
        let _ = child.kill();
    }
    CHILDREN.lock().expect("child registry").remove(&id);
    outcome
}

pub fn strip_terminal_codes(value: &str) -> String {
    static OSC: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b\][^\x07]*(?:\x07|\x1b\\)").unwrap());
    static ANSI: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|[@-_])").unwrap());
    ANSI.replace_all(&OSC.replace_all(value, ""), "")
        .replace('\r', "\n")
        .chars()
        .filter(|character| {
            matches!(*character, '\t' | '\n') || (*character >= ' ' && *character != '\u{7f}')
        })
        .collect()
}

pub fn clamp(value: f64) -> f64 {
    value.clamp(0.0, 100.0)
}

pub fn epoch_ms(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(number) => number
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .map(|v| {
                if v < 100_000_000_000.0 {
                    (v * 1000.0) as u64
                } else {
                    v as u64
                }
            }),
        serde_json::Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|date| date.timestamp_millis().max(0) as u64),
        _ => None,
    }
}

/// Reads "resets in 2h 15m" and the "refreshes in 89h 53m" wording Antigravity uses.
pub fn reset_from_text(text: &str, now: u64) -> Option<u64> {
    static KEYWORD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b(?:resets?|refreshe?s?)\b").unwrap());
    // A leading run of `<number><unit>` pairs, e.g. " in 4d 2h".
    static DURATION: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)^\s*(?:in\s+)?(?:\d+\s*(?:d(?:ays?)?|h(?:ours?|rs?)?|m(?:in(?:ute)?s?)?)\s*)+",
        )
        .unwrap()
    });
    static UNIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(\d+)\s*([dhm])").unwrap());

    if let Some(timestamp) = epoch_ms(&serde_json::Value::String(text.to_owned())) {
        return Some(timestamp);
    }
    // Only the run of units right after the keyword counts, so trailing copy cannot inflate it.
    let tail = KEYWORD
        .find(text)
        .map_or(text, |value| &text[value.end()..]);
    let minutes: u64 = UNIT
        .captures_iter(DURATION.find(tail)?.as_str())
        .map(|unit| {
            let value = unit[1].parse::<u64>().unwrap_or(0);
            match unit[2].as_bytes()[0] | 0x20 {
                b'd' => value * 1440,
                b'h' => value * 60,
                _ => value,
            }
        })
        .sum();
    (minutes > 0).then(|| now + minutes * 60_000)
}

/// Finds "Resets in 2h 15m" or "Refreshes in 89h 53m" inside a longer line.
pub fn reset_phrase(text: &str) -> Option<String> {
    static PHRASE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)(?:resets?|refreshe?s?)\s+(?:in\s+)?(?:\d+\s*[a-z]+\s*)+").unwrap()
    });
    PHRASE
        .find(text)
        .map(|value| value.as_str().trim().to_owned())
}

pub fn title_case(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

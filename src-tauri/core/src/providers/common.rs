use crate::model::{ProviderError, ProviderErrorKind};
use chrono::DateTime;
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
use regex::Regex;
use reqwest::header::{HeaderMap, RETRY_AFTER};
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

/// The children themselves, not just killers: a killed process stays in the process table as a
/// zombie until someone waits on it, and only the `Child` can do that.
static CHILDREN: LazyLock<Mutex<HashMap<u64, Box<dyn Child + Send + Sync>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_CHILD: AtomicU64 = AtomicU64::new(1);

/// A real `.exe` anywhere is preferred over an npm `.cmd` shim.
pub async fn resolve_executable(name: &str, configured: Option<&Path>) -> Option<PathBuf> {
    let shims = cfg!(windows)
        .then(|| resolve(name, &[".cmd"], None))
        .into_iter()
        .flatten();
    runnable(resolve(name, &[env::consts::EXE_SUFFIX], configured).chain(shims))
}

/// Never a `.cmd` itself, only the native exe it starts: killing a timed-out `cmd.exe` leaves the
/// `node.exe` it started running.
fn runnable(mut paths: impl Iterator<Item = PathBuf>) -> Option<PathBuf> {
    paths.find_map(|path| {
        if cfg!(windows)
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"))
        {
            shim_target(&path)
        } else {
            Some(path)
        }
    })
}

/// The native exe behind an npm cmd-shim (cmd-shim 7 or 9): a direct `.exe` target, or the one
/// `@openai/codex`'s node launcher starts. Anything else reads as not installed.
fn shim_target(shim: &Path) -> Option<PathBuf> {
    static SHIM: LazyLock<Regex> = LazyLock::new(|| {
        let pathext = regex::escape("SET PATHEXT=%PATHEXT:;.JS;=;%");
        Regex::new(&format!(
            r#"(?i)^{}(?P<node>{}(?:{pathext}\n)?\)\n{}(?:{pathext} & )?"%_prog%" +)?"%dp0%\\(?P<target>[^"\n]+)" +%\*$"#,
            regex::escape("@ECHO off\nGOTO start\n:find_dp0\nSET dp0=%~dp0\nEXIT /b\n:start\nSETLOCAL\nCALL :find_dp0\n"),
            regex::escape("IF EXIST \"%dp0%\\node.exe\" (\nSET \"_prog=%dp0%\\node.exe\"\n) ELSE (\nSET \"_prog=node\"\n"),
            regex::escape("endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "),
        ))
        .unwrap()
    });
    let mut text = String::new();
    fs::File::open(shim)
        .ok()?
        .take(64 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    let text = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let shim_match = SHIM.captures(&text)?;
    let target = &shim_match["target"];
    // A plain path below the shim's directory: no variable, drive, stream, root or `..`.
    if target.starts_with(['\\', '/'])
        || target.contains(['%', '^', '&', '|', '<', '>', ':'])
        || target.split(['\\', '/']).any(|part| part == "..")
    {
        return None;
    }
    let join = |base: &Path, relative: &str| {
        relative
            .split(['\\', '/'])
            .fold(base.to_path_buf(), |path, part| path.join(part))
    };
    let script = join(shim.parent()?, target);
    let candidates = match shim_match.name("node") {
        None if script
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")) =>
        {
            vec![script]
        }
        Some(_) if script.ends_with("@openai/codex/bin/codex.js") => {
            // ponytail: x64 and codex.js's current private layout; anything else is not installed.
            let package = script.parent()?.parent()?;
            let native = "vendor/x86_64-pc-windows-msvc/bin/codex.exe";
            vec![
                join(
                    package,
                    &format!("node_modules/@openai/codex-win32-x64/{native}"),
                ),
                join(package.parent()?, &format!("codex-win32-x64/{native}")),
                join(package, native),
                join(package, "vendor/x86_64-pc-windows-msvc/codex/codex.exe"), // before 0.99
            ]
        }
        _ => return None,
    };
    candidates.into_iter().find(|path| executable(path))
}

/// Also finds npm's `.cmd` shim, to read the package installed beside it. Never spawn the result.
pub async fn resolve_installed(name: &str, configured: Option<&Path>) -> Option<PathBuf> {
    let suffixes: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd"]
    } else {
        &[""]
    };
    resolve(name, suffixes, configured).next()
}

fn resolve<'a>(
    name: &'a str,
    suffixes: &'a [&'a str],
    configured: Option<&Path>,
) -> impl Iterator<Item = PathBuf> + 'a {
    let configured = configured
        .filter(|path| executable(path))
        .map(Path::to_path_buf);
    let mut dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(home) = home_dir() {
        dirs.extend([
            home.join(".local/bin"),
            home.join(".npm-global/bin"),
            home.join(".bun/bin"),
        ]);
    }
    configured
        .into_iter()
        .chain(find_in_dirs(name, suffixes, dirs))
}

fn find_in_dirs<'a>(
    name: &'a str,
    suffixes: &'a [&'a str],
    dirs: impl IntoIterator<Item = PathBuf> + 'a,
) -> impl Iterator<Item = PathBuf> + 'a {
    let mut seen = HashSet::new();
    dirs.into_iter()
        // A relative `PATH` entry resolves against the working directory: a planted binary.
        .filter(|dir| dir.is_absolute())
        .flat_map(move |dir| {
            suffixes
                .iter()
                .map(move |suffix| dir.join(format!("{name}{suffix}")))
        })
        .filter(move |path| seen.insert(path.clone()) && executable(path))
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
    home_from(|key| env::var_os(key))
}

fn home_from(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    // Windows CLIs use the profile; a `HOME` from Git Bash or MSYS2 can point elsewhere.
    let [first, second] = if cfg!(windows) {
        ["USERPROFILE", "HOME"]
    } else {
        ["HOME", "USERPROFILE"]
    };
    let env = |key: &str| env(key).filter(|value| !value.is_empty());
    env(first).or_else(|| env(second)).map(PathBuf::from)
}

/// An empty value counts as unset: as a path it would resolve against the working directory.
pub fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn bounded_cwd() -> Result<PathBuf, ProviderError> {
    let root = env_path("XDG_RUNTIME_DIR")
        .filter(|path| path.is_absolute())
        .map_or_else(
            || env::temp_dir().join(format!("trayci-{}", user_id())),
            |path| path.join("trayci"),
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
    // Drained before any of them is waited on: `reap` blocks, and holding the registry through a
    // wait would stall every probe still trying to register.
    let children = CHILDREN
        .lock()
        .expect("child registry")
        .drain()
        .map(|(_, child)| child)
        .collect::<Vec<_>>();
    for child in children {
        reap(child);
    }
}

/// Kills a child and collects it. In that order: the CLIs are interactive and do not exit on their
/// own, so waiting on a live one never returns. A signalled child cannot ignore SIGKILL, so the
/// wait that follows is the reap, not a second chance to hang.
fn reap(mut child: Box<dyn Child + Send + Sync>) {
    let _ = child.kill();
    let _ = child.wait();
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
        // Handed straight to the registry, so the `?`s below cannot drop it on the floor: whatever
        // this closure returns, and whether or not it is still running when the caller gives up,
        // the child is reachable to be killed and reaped.
        let id = NEXT_CHILD.fetch_add(1, Ordering::Relaxed);
        CHILDREN.lock().expect("child registry").insert(id, child);
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
    if let Some(child) = CHILDREN.lock().expect("child registry").remove(&id) {
        reap(child);
    }
    outcome
}

/// Runs a non-interactive CLI to completion and returns what it printed. The child dies with the
/// future, so a timeout, a cancellation or a dropped refresh cannot leave it running.
pub async fn run_process(
    executable: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<std::process::Output, ProviderError> {
    if cancellation.is_cancelled() {
        return Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled"));
    }
    let mut command = tokio::process::Command::new(executable);
    command
        .args(args)
        .envs(env.iter().copied())
        .current_dir(bounded_cwd()?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // CREATE_NO_WINDOW: the GUI-subsystem app has no console to lend, so Windows would open one.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let child = command.spawn().map_err(|error| {
        ProviderError::new(ProviderErrorKind::Unknown, format!("CLI start: {error}"))
    })?;
    tokio::select! {
        output = child.wait_with_output() => output.map_err(|error| ProviderError::new(ProviderErrorKind::Unknown, format!("CLI output: {error}"))),
        _ = cancellation.cancelled() => Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled")),
        _ = tokio::time::sleep(timeout) => Err(ProviderError::new(ProviderErrorKind::Timeout, "CLI probe timed out")),
    }
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

/// A 429 that waits for the server's `retry-after`, in either of its forms: delta-seconds or an
/// HTTP-date. Capped at a day, so a corrupt or hostile header cannot shelve the provider for longer.
pub fn rate_limited(headers: &HeaderMap, now: u64, message: &str) -> ProviderError {
    const DAY_MS: u64 = 24 * 60 * 60_000;
    let retry_at = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .and_then(|value| match value.parse::<u64>() {
            Ok(seconds) => Some(now + seconds.min(DAY_MS / 1000) * 1000),
            Err(_) => u64::try_from(DateTime::parse_from_rfc2822(value).ok()?.timestamp_millis())
                .ok()
                .map(|at| at.min(now + DAY_MS)),
        })
        .filter(|&at| at > now);
    ProviderError {
        retry_at,
        ..ProviderError::new(ProviderErrorKind::RateLimited, message)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::consts::EXE_SUFFIX;

    fn install(dir: &Path, file_name: &str) -> PathBuf {
        let path = dir.join(file_name);
        fs::write(&path, b"stub").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    #[test]
    fn finds_the_cli_the_way_the_platform_installs_it() {
        let dir = tempfile::tempdir().unwrap();
        let installed = install(dir.path(), &format!("claude{EXE_SUFFIX}"));

        assert_eq!(
            find_in_dirs("claude", &[EXE_SUFFIX], [dir.path().to_path_buf()]).next(),
            Some(installed)
        );
    }

    #[test]
    fn prefers_the_earlier_directory() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let expected = install(first.path(), &format!("claude{EXE_SUFFIX}"));
        install(second.path(), &format!("claude{EXE_SUFFIX}"));

        assert_eq!(
            find_in_dirs(
                "claude",
                &[EXE_SUFFIX],
                [first.path().to_path_buf(), second.path().to_path_buf()]
            )
            .next(),
            Some(expected)
        );
    }

    #[test]
    fn home_prefers_the_platform_variable_and_skips_an_empty_one() {
        let resolve = |home: &str, userprofile: &str| {
            home_from(|key| match key {
                "HOME" => Some(home.into()),
                "USERPROFILE" => Some(userprofile.into()),
                _ => None,
            })
        };

        let preferred = if cfg!(windows) { "/profile" } else { "/home" };
        assert_eq!(resolve("/home", "/profile"), Some(PathBuf::from(preferred)));
        // Each host reads a different variable first, so each assert covers one of them.
        assert_eq!(resolve("", "/profile"), Some(PathBuf::from("/profile")));
        assert_eq!(resolve("/home", ""), Some(PathBuf::from("/home")));
        assert_eq!(resolve("", ""), None);
    }

    #[test]
    fn reports_nothing_when_no_directory_has_it() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(
            find_in_dirs("claude", &[EXE_SUFFIX], [dir.path().to_path_buf()]).next(),
            None
        );
    }

    #[test]
    fn skips_a_relative_directory() {
        let cwd = env::current_dir().unwrap();
        let dir = tempfile::tempdir_in(&cwd).unwrap();
        install(dir.path(), &format!("claude{EXE_SUFFIX}"));
        let relative = dir.path().strip_prefix(&cwd).unwrap().to_path_buf();

        assert_eq!(
            find_in_dirs("claude", &[EXE_SUFFIX], [relative]).next(),
            None
        );
    }

    #[cfg(windows)]
    #[test]
    fn ignores_the_extensionless_script_npm_leaves_beside_a_cmd_shim() {
        let dir = tempfile::tempdir().unwrap();
        install(dir.path(), "codex");
        install(dir.path(), "codex.cmd");

        assert_eq!(
            find_in_dirs("codex", &[EXE_SUFFIX], [dir.path().to_path_buf()]).next(),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn skips_a_file_that_is_not_executable() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("claude"), b"stub").unwrap();

        assert_eq!(
            find_in_dirs("claude", &[EXE_SUFFIX], [dir.path().to_path_buf()]).next(),
            None
        );
    }

    #[test]
    fn reads_the_gemini_client_beside_an_npm_cmd_shim() {
        // `npm install -g` on Windows: shims in `%APPDATA%\npm`, the package in `node_modules` there.
        let npm = tempfile::tempdir().unwrap();
        let package = npm.path().join("node_modules/@google/gemini-cli");
        fs::create_dir_all(package.join("bundle")).unwrap();
        fs::write(
            package.join("package.json"),
            r#"{"name":"@google/gemini-cli"}"#,
        )
        .unwrap();
        fs::write(
            package.join("bundle/chunk-UF44X63P.js"),
            r#"var OAUTH_CLIENT_ID = "id"; var OAUTH_CLIENT_SECRET = "secret";"#,
        )
        .unwrap();
        for shim in ["gemini", "gemini.cmd", "gemini.ps1"] {
            install(npm.path(), shim);
        }
        let dirs = || [npm.path().to_path_buf()];

        // What `resolve_executable` searches on Windows: a spawned CLI never resolves to a shim.
        assert_eq!(find_in_dirs("gemini", &[".exe"], dirs()).next(), None);
        let shim = find_in_dirs("gemini", &[".exe", ".cmd"], dirs())
            .next()
            .unwrap();
        assert_eq!(shim, npm.path().join("gemini.cmd"));
        assert_eq!(
            super::super::google_oauth::from_binary(shim),
            Some(("id".into(), "secret".into()))
        );
    }

    const SHIM_HEAD: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n";

    fn direct_shim(target: &str) -> String {
        format!("{SHIM_HEAD}\"%dp0%\\{target}\"   %*\r\n")
    }

    /// cmd-shim 7 (npm 10 and 11) sets PATHEXT in the ELSE branch; cmd-shim 9 on the last line.
    fn node_shim(script: &str, cmd_shim_9: bool) -> String {
        let (branch, last) = match cmd_shim_9 {
            false => ("  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n", ""),
            true => ("", "set PATHEXT=%PATHEXT:;.JS;=;% & "),
        };
        format!(
            "{SHIM_HEAD}\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n{branch})\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & {last}\"%_prog%\"  \"%dp0%\\{script}\" %*\r\n"
        )
    }

    fn write_shim(dir: &Path, contents: &str) -> PathBuf {
        let shim = install(dir, "tool.cmd");
        fs::write(&shim, contents).unwrap();
        shim
    }

    fn install_claude(dir: &Path) -> (PathBuf, PathBuf) {
        let bin = dir.join("node_modules/@anthropic-ai/claude-code/bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = install(&bin, "claude.exe");
        let shim = install(dir, "claude.cmd");
        fs::write(
            &shim,
            direct_shim(r"node_modules\@anthropic-ai\claude-code\bin\claude.exe"),
        )
        .unwrap();
        (shim, exe)
    }

    #[tokio::test]
    async fn runs_the_exe_an_npm_shim_starts_never_the_shim() {
        let dir = tempfile::tempdir().unwrap();
        let (shim, exe) = install_claude(dir.path());
        // Elsewhere a script named `*.cmd` is just a script.
        let expected = if cfg!(windows) { exe } else { shim.clone() };

        assert_eq!(
            resolve_executable("claude", Some(&shim)).await,
            Some(expected)
        );
    }

    #[cfg(windows)]
    #[test]
    fn looks_past_a_shim_it_cannot_follow() {
        let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        fs::write(
            install(first.path(), "claude.cmd"),
            "@ECHO off\r\nnode cli.js %*\r\n",
        )
        .unwrap();
        let (_, exe) = install_claude(second.path());
        let dirs = [first.path().to_path_buf(), second.path().to_path_buf()];

        assert_eq!(runnable(find_in_dirs("claude", &[".cmd"], dirs)), Some(exe));
    }

    #[test]
    fn follows_the_codex_shim_to_its_native_binary() {
        let native = "vendor/x86_64-pc-windows-msvc/bin/codex.exe";
        for layout in [
            format!("node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/{native}"),
            format!("node_modules/@openai/codex-win32-x64/{native}"),
            format!("node_modules/@openai/codex/{native}"),
            "node_modules/@openai/codex/vendor/x86_64-pc-windows-msvc/codex/codex.exe".into(),
        ] {
            for cmd_shim_9 in [false, true] {
                let dir = tempfile::tempdir().unwrap();
                let exe = dir.path().join(&layout);
                fs::create_dir_all(exe.parent().unwrap()).unwrap();
                install(exe.parent().unwrap(), "codex.exe");
                let shim = write_shim(
                    dir.path(),
                    &node_shim(r"node_modules\@openai\codex\bin\codex.js", cmd_shim_9),
                );

                assert_eq!(shim_target(&shim), Some(exe), "{layout}");
            }
        }
    }

    #[test]
    fn rejects_shims_it_does_not_recognise() {
        let dir = tempfile::tempdir().unwrap();
        let exe = install(dir.path(), "tool.exe");
        install(dir.path(), "tool.js");
        // Each rejected target exists, so only the rule rejects it.
        for sub in ["%TEMP%", "sub"] {
            fs::create_dir(dir.path().join(sub)).unwrap();
            install(&dir.path().join(sub), "tool.exe");
        }
        let rejected = [
            direct_shim("tool.js"),
            node_shim("tool.js", false),
            direct_shim(r"%TEMP%\tool.exe"),
            direct_shim(r"sub\..\tool.exe"),
            direct_shim(r"\tool.exe"),
            direct_shim("missing.exe"),
            direct_shim("tool.exe")
                .replace("CALL :find_dp0\r\n", "CALL :find_dp0\r\nSET PATH=x\r\n"),
        ];

        assert_eq!(
            shim_target(&write_shim(dir.path(), &direct_shim("tool.exe"))),
            Some(exe)
        );
        for contents in rejected {
            assert_eq!(
                shim_target(&write_shim(dir.path(), &contents)),
                None,
                "{contents}"
            );
        }
    }
}

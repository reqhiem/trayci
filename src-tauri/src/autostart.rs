use std::io;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
fn path() -> PathBuf {
    trayci_core::settings::config_base().join("autostart/trayci.desktop")
}

/// This executable as a new process should start it: the AppImage itself rather than its mount,
/// which goes away with the process that mounted it.
#[cfg(target_os = "linux")]
pub fn executable() -> io::Result<std::path::PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(std::path::PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)
}

#[cfg(target_os = "linux")]
pub async fn set_enabled(enabled: bool) -> io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let path = path();
    if !enabled {
        return match tokio::fs::remove_file(path).await {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }

    let executable = executable()?;
    let escaped = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Trayci\nComment=AI coding usage in your system tray\nExec=\"{escaped}\" tray\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    let parent = path.parent().expect("autostart path has a parent");
    tokio::fs::create_dir_all(parent).await?;
    tokio::fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700)).await?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = tokio::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true).mode(0o600);
    let mut file = options.open(&temporary).await?;
    file.write_all(contents.as_bytes()).await?;
    file.sync_all().await?;
    tokio::fs::rename(temporary, path).await
}

/// Named after `productName`: the NSIS uninstaller deletes exactly this Run value.
#[cfg(windows)]
pub async fn set_enabled(enabled: bool) -> io::Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};

    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run")?;
    if !enabled {
        return match key.delete_value("Trayci") {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        };
    }
    let executable = std::env::current_exe()?;
    key.set_value("Trayci", &format!("\"{}\" tray", executable.display()))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn set_enabled(_enabled: bool) -> io::Result<()> {
    Ok(())
}

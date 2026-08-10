use std::io;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
fn path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("autostart/trayci.desktop")
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

    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)?;
    let escaped = executable
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Trayci\nComment=AI coding usage in your system tray\nExec=\"{escaped}\"\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
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

#[cfg(not(target_os = "linux"))]
pub async fn set_enabled(_enabled: bool) -> io::Result<()> {
    Ok(())
}

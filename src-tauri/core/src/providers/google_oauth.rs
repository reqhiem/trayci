use super::common::resolve_executable;
use regex::Regex;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

fn parse(source: &str) -> Option<(String, String)> {
    static ID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"OAUTH_CLIENT_ID\s*=\s*["']([^"']+)["']"#).unwrap());
    static SECRET: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"OAUTH_CLIENT_SECRET\s*=\s*["']([^"']+)["']"#).unwrap());
    Some((
        ID.captures(source)?.get(1)?.as_str().into(),
        SECRET.captures(source)?.get(1)?.as_str().into(),
    ))
}

fn read(path: &Path) -> Option<(String, String)> {
    parse(&fs::read_to_string(path).ok()?)
}

fn package_root(binary: &Path) -> Option<PathBuf> {
    let mut current = binary.parent()?.to_path_buf();
    for _ in 0..=8 {
        if fs::read(current.join("package.json"))
            .ok()
            .and_then(|data| serde_json::from_slice::<Value>(&data).ok())
            .and_then(|value| value.get("name").and_then(Value::as_str).map(str::to_owned))
            .as_deref()
            == Some("@google/gemini-cli")
        {
            return Some(current);
        }
        for nested in [
            current.join("lib/node_modules/@google/gemini-cli"),
            current.join("node_modules/@google/gemini-cli"),
        ] {
            if nested.join("package.json").is_file() {
                return Some(nested);
            }
        }
        if !current.pop() {
            break;
        }
    }
    None
}

pub async fn extract(configured: Option<&Path>) -> Option<(String, String)> {
    let binary = resolve_executable("gemini", configured).await?;
    let resolved = fs::canonicalize(&binary).unwrap_or(binary);
    let bin = resolved.parent()?;
    let prefix = bin.parent()?;
    const SOURCE: &str = "dist/src/code_assist/oauth2.js";
    for candidate in [
        prefix
            .join(
                "libexec/lib/node_modules/@google/gemini-cli/node_modules/@google/gemini-cli-core",
            )
            .join(SOURCE),
        prefix
            .join("lib/node_modules/@google/gemini-cli/node_modules/@google/gemini-cli-core")
            .join(SOURCE),
        prefix.join("../gemini-cli-core").join(SOURCE),
        prefix
            .join("node_modules/@google/gemini-cli-core")
            .join(SOURCE),
    ] {
        if let Some(value) = read(&candidate) {
            return Some(value);
        }
    }
    let root = package_root(&resolved)?;
    for candidate in [
        root.join("node_modules/@google/gemini-cli-core")
            .join(SOURCE),
        root.join(SOURCE),
    ] {
        if let Some(value) = read(&candidate) {
            return Some(value);
        }
    }
    for entry in fs::read_dir(root.join("bundle")).ok()? {
        let path = entry.ok()?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("js") {
            if let Some(value) = read(&path) {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_client_constants() {
        assert_eq!(
            super::parse("const OAUTH_CLIENT_ID = 'id'; const OAUTH_CLIENT_SECRET=\"secret\";"),
            Some(("id".into(), "secret".into()))
        );
    }
}

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{collections::BTreeMap, fs::File, io::Read, os::unix::fs::MetadataExt, path::Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub url: String,
    pub username: String,
    pub password: String,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub protected_ports: Vec<u64>,
    /// Friendly names keyed by full MAC address; labels do not prove attachment.
    #[serde(default)]
    pub device_names: BTreeMap<String, String>,
}

fn timeout() -> u64 {
    15
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        // Check the opened file, not a separate path stat (avoids a permission-check race).
        let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let meta = file.metadata()?;
        ensure!(meta.is_file(), "credentials must be a regular file");
        ensure!(
            meta.mode() & 0o077 == 0,
            "credentials must not be accessible by group/others; run chmod 600 {}",
            path.display()
        );
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        // Parser errors can contain source snippets; never print credential file contents.
        let mut config: Self = toml::from_str(&text)
            .map_err(|_| anyhow::anyhow!("invalid credentials TOML (contents redacted)"))?;
        ensure!(
            !config.username.is_empty() && !config.password.is_empty(),
            "username and password cannot be empty"
        );
        ensure!(
            (1..=120).contains(&config.timeout_seconds),
            "timeout_seconds must be 1..120"
        );
        let url = reqwest::Url::parse(&config.url).context("invalid switch URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "switch URL must use HTTP or HTTPS"
        );
        ensure!(
            url.username().is_empty() && url.password().is_none(),
            "put credentials in TOML fields, not URL"
        );
        ensure!(
            url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
            "switch URL must be an origin, without path/query/fragment"
        );
        let mut names = BTreeMap::new();
        for (mac, name) in config.device_names {
            let mac =
                crate::identity::normalize_mac(&mac).context("invalid device_names MAC key")?;
            ensure!(
                !name.trim().is_empty(),
                "device_names labels cannot be empty"
            );
            ensure!(
                names.insert(mac, name).is_none(),
                "duplicate normalized device_names MAC key"
            );
        }
        config.device_names = names;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};
    #[test]
    fn credentials_permissions_and_redaction() {
        let f = tempfile::NamedTempFile::new().unwrap();
        fs::write(
            f.path(),
            "url = 'http://localhost'\nusername = 'admin'\npassword = 'secret'\n",
        )
        .unwrap();
        assert!(Config::load(f.path()).is_ok());
        fs::set_permissions(f.path(), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Config::load(f.path()).is_err());
        fs::set_permissions(f.path(), fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(f.path(), "password = 'secret\n").unwrap();
        assert!(
            !Config::load(f.path())
                .err()
                .unwrap()
                .to_string()
                .contains("secret")
        );
    }
}

//! The persisted allowlist configuration.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

pub const CONFIG_VERSION: u32 = 1;
pub const DEFAULT_PORT: u16 = 47851;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub allowed_apps: Vec<AllowedApp>,
    #[serde(default)]
    pub http: HttpConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            allowed_apps: Vec::new(),
            http: HttpConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowedApp {
    pub id: String,
    pub name: String,
    pub identity: AppIdentity,
}

impl AllowedApp {
    /// A new entry with a fresh `app_<uuid>` id.
    pub fn new(name: impl Into<String>, identity: AppIdentity) -> Self {
        Self {
            id: format!("app_{}", uuid::Uuid::new_v4()),
            name: name.into(),
            identity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "lowercase",
    rename_all_fields = "camelCase"
)]
pub enum AppIdentity {
    /// An executable, matched by canonical path.
    Exe { path: String },
    /// A Windows Store package, matched by package family name.
    Package { family_name: String },
    /// A macOS `.app` bundle, matched by the executables inside it.
    Bundle { path: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HttpConfig {
    pub enabled: bool,
    pub port: u16,
    pub token: String,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            token: String::new(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    /// The file exists but is not a valid config. The file is left untouched.
    Invalid(String),
    Io(std::io::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Invalid(m) => write!(f, "invalid config: {m}"),
            ConfigError::Io(e) => write!(f, "config i/o error: {e}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e)
    }
}

/// 32 random bytes, base64url without padding.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the operating system random source is unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Loads the config at `path`.
///
/// A missing file gives the default config. Either way an empty HTTP token is
/// replaced with a freshly generated one in the returned value (nothing is
/// written). Invalid JSON gives [`ConfigError::Invalid`] and the file is never
/// overwritten, so the user's allowlist survives a typo.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let mut config = match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Config>(&text)
            .map_err(|e| ConfigError::Invalid(e.to_string()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => return Err(e.into()),
    };
    if config.http.token.is_empty() {
        config.http.token = generate_token();
    }
    Ok(config)
}

/// Writes the config atomically: `config.json.tmp`, fsync, then rename. On
/// unix the file is created with mode 0600 because it holds the HTTP token.
pub fn save(path: &Path, config: &Config) -> Result<(), ConfigError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let body =
        serde_json::to_vec_pretty(config).map_err(|e| ConfigError::Invalid(e.to_string()))?;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&body)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Problems that make a config unusable. Empty means valid.
pub fn validate(config: &Config) -> Vec<String> {
    let mut problems = Vec::new();
    if config.version != CONFIG_VERSION {
        problems.push(format!(
            "unsupported config version {} (expected {CONFIG_VERSION})",
            config.version
        ));
    }
    if config.http.port == 0 {
        problems.push("http.port must not be 0".to_string());
    }
    let mut seen = std::collections::HashSet::new();
    for app in &config.allowed_apps {
        if !seen.insert(app.id.as_str()) {
            problems.push(format!("duplicate app id {}", app.id));
        }
        let empty = match &app.identity {
            AppIdentity::Exe { path } | AppIdentity::Bundle { path } => path.trim().is_empty(),
            AppIdentity::Package { family_name } => family_name.trim().is_empty(),
        };
        if empty {
            problems.push(format!("app {} has an empty identity", app.id));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_round_trips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut config = Config::default();
        config.http.token = "tok".into();
        config.allowed_apps.push(AllowedApp::new(
            "Example",
            AppIdentity::Exe {
                path: "C:\\x.exe".into(),
            },
        ));
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap(), config);
        assert!(!dir.path().join("config.json.tmp").exists());
    }

    #[test]
    fn allowed_apps_serialises_as_camel_case() {
        let value = serde_json::to_value(Config::default()).unwrap();
        assert!(value.get("allowedApps").is_some());
        assert!(value.get("allowed_apps").is_none());
        assert_eq!(value["http"]["port"], 47851);
        assert_eq!(value["http"]["enabled"], false);
    }

    #[test]
    fn exe_identity_parses() {
        let identity: AppIdentity =
            serde_json::from_value(json!({"kind":"exe","path":"C:\\x.exe"})).unwrap();
        assert_eq!(
            identity,
            AppIdentity::Exe {
                path: "C:\\x.exe".into()
            }
        );
    }

    #[test]
    fn package_and_bundle_identities_parse() {
        let package: AppIdentity =
            serde_json::from_value(json!({"kind":"package","familyName":"Vendor.App_abc"}))
                .unwrap();
        assert_eq!(
            package,
            AppIdentity::Package {
                family_name: "Vendor.App_abc".into()
            }
        );
        let bundle: AppIdentity =
            serde_json::from_value(json!({"kind":"bundle","path":"/Applications/X.app"})).unwrap();
        assert_eq!(
            bundle,
            AppIdentity::Bundle {
                path: "/Applications/X.app".into()
            }
        );
    }

    #[test]
    fn missing_file_gives_default_with_a_generated_token() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(&dir.path().join("config.json")).unwrap();
        assert!(config.allowed_apps.is_empty());
        assert_eq!(config.http.port, DEFAULT_PORT);
        assert!(!config.http.enabled);
        // 32 bytes of base64url without padding.
        assert_eq!(config.http.token.len(), 43);
        assert!(!dir.path().join("config.json").exists());
    }

    #[test]
    fn generated_tokens_differ() {
        assert_ne!(generate_token(), generate_token());
    }

    #[test]
    fn corrupt_json_is_invalid_and_the_file_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(load(&path), Err(ConfigError::Invalid(_))));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn save_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.json");
        let mut config = Config::default();
        config.http.token = "one".into();
        save(&path, &config).unwrap();
        config.http.token = "two".into();
        save(&path, &config).unwrap();
        assert_eq!(load(&path).unwrap().http.token, "two");
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_private_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        save(&path, &Config::default()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn validate_flags_each_problem() {
        let mut config = Config::default();
        assert!(validate(&config).is_empty());
        config.version = 2;
        config.http.port = 0;
        let app = AllowedApp {
            id: "app_1".into(),
            name: "A".into(),
            identity: AppIdentity::Exe { path: " ".into() },
        };
        config.allowed_apps = vec![app.clone(), app];
        let problems = validate(&config);
        assert_eq!(problems.len(), 5, "{problems:?}");
    }
}

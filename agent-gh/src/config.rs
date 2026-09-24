use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use serde::Deserialize;
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::num::NonZeroU64;
use std::path::PathBuf;

const CONFIG_ENV: &str = "AGENT_GH_CONFIG";
const CACHE_DIR_ENV: &str = "AGENT_GH_CACHE_DIR";
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";
const APP_DIR: &str = "agent-gh";

pub(crate) struct Paths {
    pub(crate) config: Utf8PathBuf,
    pub(crate) cache: Utf8PathBuf,
}

impl Paths {
    pub(crate) fn from_env() -> Result<Self> {
        let config = match non_empty_var(CONFIG_ENV) {
            Some(value) => absolute(value, CONFIG_ENV)?,
            None => config_home(non_empty_var(XDG_CONFIG_HOME_ENV), dirs::home_dir())?
                .join(APP_DIR)
                .join("config.toml"),
        };
        let cache_dir = match non_empty_var(CACHE_DIR_ENV) {
            Some(value) => absolute(value, CACHE_DIR_ENV)?,
            None => platform_dir(dirs::cache_dir(), "cache")?.join(APP_DIR),
        };
        Ok(Self {
            config,
            cache: cache_dir.join("token.toml"),
        })
    }
}

pub(crate) struct Config {
    pub(crate) app_id: NonZeroU64,
    pub(crate) installation_id: NonZeroU64,
    pub(crate) private_key_path: Utf8PathBuf,
    pub(crate) run_as_user: Vec<CommandPrefix>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommandPrefix(Vec<OsString>);

impl CommandPrefix {
    fn parse(entry: &str) -> Result<Self> {
        let words: Vec<OsString> = entry.split_whitespace().map(OsString::from).collect();
        if words.is_empty() {
            bail!("run_as_user entry {entry:?} must contain a command");
        }
        Ok(Self(words))
    }

    pub(crate) fn matches(&self, args: &[OsString]) -> bool {
        args.starts_with(&self.0)
    }
}

impl fmt::Display for CommandPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0.join(OsStr::new(" ")).display())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    app_id: NonZeroU64,
    installation_id: NonZeroU64,
    private_key_path: String,
    #[serde(default)]
    run_as_user: Vec<String>,
}

impl Config {
    pub(crate) fn load(path: &Utf8Path) -> Result<Self> {
        let text = fs_err::read_to_string(path)?;
        let base_dir = path.parent().unwrap_or(Utf8Path::new(""));
        Self::parse(&text, base_dir).with_context(|| format!("parsing {path}"))
    }

    fn parse(text: &str, base_dir: &Utf8Path) -> Result<Self> {
        let file: ConfigFile = toml::from_str(text)?;
        if file.private_key_path.is_empty() {
            bail!("private_key_path must not be empty");
        }
        let run_as_user = file
            .run_as_user
            .iter()
            .map(|entry| CommandPrefix::parse(entry))
            .collect::<Result<_>>()?;
        Ok(Self {
            app_id: file.app_id,
            installation_id: file.installation_id,
            private_key_path: base_dir.join(file.private_key_path),
            run_as_user,
        })
    }
}

fn non_empty_var(name: &str) -> Option<OsString> {
    env::var_os(name).filter(|value| !value.is_empty())
}

fn absolute(value: OsString, name: &str) -> Result<Utf8PathBuf> {
    let path = std::path::absolute(value)
        .with_context(|| format!("resolving {name} against the working directory"))?;
    Utf8PathBuf::try_from(path).with_context(|| format!("{name} is not valid UTF-8"))
}

fn config_home(xdg_config_home: Option<OsString>, home: Option<PathBuf>) -> Result<Utf8PathBuf> {
    let dir = match xdg_config_home
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
    {
        Some(dir) => dir,
        None => home
            .context("the user has no home directory")?
            .join(".config"),
    };
    Utf8PathBuf::try_from(dir).context("the configuration directory is not valid UTF-8")
}

fn platform_dir(dir: Option<PathBuf>, kind: &str) -> Result<Utf8PathBuf> {
    let dir = dir.with_context(|| format!("the platform has no user {kind} directory"))?;
    Utf8PathBuf::try_from(dir)
        .with_context(|| format!("the user {kind} directory is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "/home/agent/.config/agent-gh";

    fn parse(text: &str) -> Result<Config> {
        Config::parse(text, Utf8Path::new(BASE))
    }

    fn error(text: &str) -> String {
        match parse(text) {
            Ok(_) => panic!("configuration should be rejected:\n{text}"),
            Err(error) => format!("{error:#}"),
        }
    }

    #[test]
    fn resolves_relative_key_path_against_config_directory() {
        let config = parse(
            "app_id = 1234567\ninstallation_id = 98765432\nprivate_key_path = \"keys/app.pem\"\n",
        )
        .expect("configuration is valid");
        assert_eq!(config.app_id.get(), 1_234_567);
        assert_eq!(config.installation_id.get(), 98_765_432);
        assert_eq!(
            config.private_key_path,
            Utf8Path::new(BASE).join("keys/app.pem")
        );
    }

    #[test]
    fn keeps_absolute_key_path() {
        let key = std::env::temp_dir().join("app.pem");
        let key = key.to_str().expect("temporary directory is UTF-8");
        let text = format!("app_id = 1\ninstallation_id = 2\nprivate_key_path = '{key}'\n");
        let config = parse(&text).expect("configuration is valid");
        assert_eq!(config.private_key_path, Utf8Path::new(key));
    }

    #[test]
    fn rejects_zero_ids() {
        let message = error("app_id = 0\ninstallation_id = 2\nprivate_key_path = \"app.pem\"\n");
        assert!(message.contains("nonzero"), "{message}");
    }

    #[test]
    fn rejects_negative_ids() {
        error("app_id = 1\ninstallation_id = -2\nprivate_key_path = \"app.pem\"\n");
    }

    #[test]
    fn rejects_unknown_fields() {
        let message = error(
            "app_id = 1\ninstallation_id = 2\nprivate_key_path = \"app.pem\"\nrepository_ids = [3]\n",
        );
        assert!(message.contains("repository_ids"), "{message}");
    }

    #[test]
    fn rejects_missing_fields() {
        let message = error("app_id = 1\nprivate_key_path = \"app.pem\"\n");
        assert!(message.contains("installation_id"), "{message}");
    }

    #[test]
    fn rejects_empty_key_path() {
        let message = error("app_id = 1\ninstallation_id = 2\nprivate_key_path = \"\"\n");
        assert!(message.contains("private_key_path"), "{message}");
    }

    #[test]
    fn splits_run_as_user_entries_into_words() {
        let config = parse(
            "app_id = 1\ninstallation_id = 2\nprivate_key_path = \"app.pem\"\nrun_as_user = [\"pr create\", \" pr \t new \"]\n",
        )
        .expect("configuration is valid");
        assert_eq!(
            config.run_as_user,
            [
                CommandPrefix(vec!["pr".into(), "create".into()]),
                CommandPrefix(vec!["pr".into(), "new".into()]),
            ]
        );
    }

    #[test]
    fn defaults_run_as_user_to_empty() {
        let config = parse("app_id = 1\ninstallation_id = 2\nprivate_key_path = \"app.pem\"\n")
            .expect("configuration is valid");
        assert!(config.run_as_user.is_empty());
    }

    #[test]
    fn rejects_blank_run_as_user_entries() {
        let cases = [
            ("", r#"run_as_user entry "" must contain a command"#),
            (" \t ", r#"run_as_user entry " \t " must contain a command"#),
        ];
        for (entry, expected) in cases {
            let message = error(&format!(
                "app_id = 1\ninstallation_id = 2\nprivate_key_path = \"app.pem\"\nrun_as_user = [\"pr create\", \"{entry}\"]\n"
            ));
            assert_eq!(message, expected, "{entry:?}");
        }
    }

    #[test]
    fn command_prefix_matches_equal_leading_arguments() {
        let prefix = CommandPrefix::parse("pr create").expect("entry contains a command");
        let cases: &[(&[&str], bool)] = &[
            (&["pr", "create"], true),
            (&["pr", "create", "--fill"], true),
            (&["pr"], false),
            (&["pr", "comment", "1"], false),
            (&["pr", "--repo", "owner/repo", "create"], false),
            (&["PR", "create"], false),
            (&["pr-create"], false),
            (&["issue", "create"], false),
        ];
        for (args, expected) in cases {
            let args: Vec<OsString> = args.iter().map(OsString::from).collect();
            assert_eq!(prefix.matches(&args), *expected, "{args:?}");
        }
    }

    #[test]
    fn command_prefix_displays_words_separated_by_spaces() {
        let prefix = CommandPrefix::parse(" pr \t create ").expect("entry contains a command");
        assert_eq!(prefix.to_string(), "pr create");
    }

    #[test]
    fn defaults_config_home_to_dot_config_in_home() {
        let home = std::env::temp_dir();
        for xdg_config_home in [None, Some(""), Some("relative/config")] {
            let dir = config_home(xdg_config_home.map(OsString::from), Some(home.clone()))
                .expect("home directory is UTF-8");
            assert_eq!(
                dir.as_std_path(),
                home.join(".config"),
                "{xdg_config_home:?}"
            );
        }
    }

    #[test]
    fn uses_absolute_xdg_config_home() {
        let xdg_config_home = std::env::temp_dir().join("xdg-config");
        let dir = config_home(
            Some(xdg_config_home.clone().into_os_string()),
            Some(PathBuf::from("/home/agent")),
        )
        .expect("XDG_CONFIG_HOME is UTF-8");
        assert_eq!(dir.as_std_path(), xdg_config_home);
    }

    #[test]
    fn reports_missing_home_directory() {
        let result = config_home(None, None).map(Utf8PathBuf::into_string);
        assert!(result.is_err(), "{result:?}");
    }

    #[test]
    fn resolves_relative_override_against_working_directory() {
        let path = absolute(OsString::from("custom/config.toml"), CONFIG_ENV)
            .expect("working directory is UTF-8");
        let expected = std::env::current_dir()
            .expect("working directory is readable")
            .join("custom/config.toml");
        assert_eq!(path.as_std_path(), expected);
    }
}

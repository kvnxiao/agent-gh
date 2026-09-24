use crate::config::Profile;
use crate::github::GitHub;
use anyhow::Context;
use anyhow::Result;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use jiff::SignedDuration;
use jiff::Timestamp;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io;
use std::io::Write;
use std::num::NonZeroU64;

const REFRESH_MARGIN: SignedDuration = SignedDuration::from_secs(5 * 60);

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CachedToken {
    pub(crate) app_id: NonZeroU64,
    pub(crate) installation_id: NonZeroU64,
    pub(crate) token: String,
    pub(crate) expires_at: Timestamp,
}

impl CachedToken {
    fn belongs_to(&self, profile: &Profile) -> bool {
        self.app_id == profile.app_id && self.installation_id == profile.installation_id
    }

    fn fresh_at(&self, now: Timestamp) -> bool {
        now.checked_add(REFRESH_MARGIN)
            .is_ok_and(|limit| self.expires_at > limit)
    }
}

pub(crate) fn token_path(cache_dir: &Utf8Path, profile: &Profile) -> Utf8PathBuf {
    cache_dir.join(format!(
        "token-{}-{}.toml",
        profile.app_id, profile.installation_id
    ))
}

pub(crate) fn obtain(
    profile: &Profile,
    cache_dir: &Utf8Path,
    github: &GitHub,
    now: Timestamp,
) -> Result<CachedToken> {
    if let Some(cached) = read(cache_dir, profile)?
        && cached.belongs_to(profile)
        && cached.fresh_at(now)
    {
        return Ok(cached);
    }
    refresh(profile, cache_dir, github, now)
}

pub(crate) fn refresh(
    profile: &Profile,
    cache_dir: &Utf8Path,
    github: &GitHub,
    now: Timestamp,
) -> Result<CachedToken> {
    let issued = github.create_installation_token(profile, now)?;
    let cached = CachedToken {
        app_id: profile.app_id,
        installation_id: profile.installation_id,
        token: issued.token,
        expires_at: issued.expires_at,
    };
    write_toml(&token_path(cache_dir, profile), &cached).context("writing the token cache")?;
    Ok(cached)
}

pub(crate) fn read(cache_dir: &Utf8Path, profile: &Profile) -> Result<Option<CachedToken>> {
    read_toml(&token_path(cache_dir, profile))
}

pub(crate) fn describe(cached: Option<&CachedToken>, profile: &Profile, now: Timestamp) -> String {
    match cached {
        None => "none cached".to_owned(),
        Some(cached) if !cached.belongs_to(profile) => {
            "cached for a different App or installation".to_owned()
        }
        Some(cached) if cached.expires_at <= now => format!("expired at {}", cached.expires_at),
        Some(cached) if !cached.fresh_at(now) => {
            format!(
                "expires at {}; the next gh command refreshes it",
                cached.expires_at
            )
        }
        Some(cached) => format!("valid until {}", cached.expires_at),
    }
}

pub(crate) fn read_toml<T: DeserializeOwned>(path: &Utf8Path) -> Result<Option<T>> {
    match fs_err::read(path) {
        Ok(bytes) => Ok(str::from_utf8(&bytes)
            .ok()
            .and_then(|text| toml::from_str(text).ok())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn write_toml(path: &Utf8Path, value: &impl Serialize) -> Result<()> {
    let dir = path
        .parent()
        .context("the cache path has no parent directory")?;
    let name = path
        .file_name()
        .context("the cache path has no file name")?;
    create_private_dir(dir)?;
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    write_private_file(&temp, toml::to_string(value)?.as_bytes())?;
    fs_err::rename(&temp, path)?;
    Ok(())
}

fn create_private_dir(dir: &Utf8Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .with_context(|| format!("creating {dir}"))
}

fn write_private_file(path: &Utf8Path, contents: &[u8]) -> Result<()> {
    let mut options = fs_err::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use fs_err::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use crate::test_support::Response;
    use tempfile::TempDir;

    const NOW: &str = "2026-09-24T12:00:00Z";
    const UNREACHABLE_API: &str = "http://127.0.0.1:9";

    struct Fixture {
        dir: TempDir,
        profile: Profile,
        cache_dir: Utf8PathBuf,
    }

    impl Fixture {
        fn token_file(&self) -> Utf8PathBuf {
            self.cache_dir.join("token-1-2.toml")
        }

        fn stored(&self) -> CachedToken {
            read_toml(&self.token_file())
                .expect("cache is readable")
                .expect("cache holds a token")
        }
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temporary directory is created");
        let root =
            Utf8PathBuf::try_from(dir.path().to_path_buf()).expect("temporary directory is UTF-8");
        Fixture {
            profile: test_support::profile(&dir, 1, 2),
            cache_dir: root.join("cache"),
            dir,
        }
    }

    fn id(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).expect("fixture ID is nonzero")
    }

    fn at(timestamp: &str) -> Timestamp {
        timestamp.parse().expect("fixture timestamp is valid")
    }

    fn seed(path: &Utf8Path, app_id: u64, installation_id: u64, expires_at: &str) {
        fs_err::create_dir_all(path.parent().expect("cache path has a parent"))
            .expect("cache directory is created");
        let text = format!(
            "app_id = {app_id}\ninstallation_id = {installation_id}\ntoken = \"cached-token\"\nexpires_at = \"{expires_at}\"\n"
        );
        fs_err::write(path, text).expect("cache file is written");
    }

    fn issued(server_expiry: &str) -> test_support::Server {
        test_support::serve(vec![Response::json(
            201,
            format!(r#"{{"token":"new-token","expires_at":"{server_expiry}"}}"#),
        )])
    }

    #[test]
    fn names_token_file_after_app_and_installation() {
        let fixture = fixture();
        assert_eq!(
            token_path(&fixture.cache_dir, &fixture.profile),
            fixture.cache_dir.join("token-1-2.toml")
        );
    }

    #[test]
    fn reuses_fresh_matching_token_without_network_requests() {
        let fixture = fixture();
        seed(&fixture.token_file(), 1, 2, "2026-09-24T12:30:00Z");
        let github = GitHub::with_api_url(UNREACHABLE_API);

        let token = obtain(&fixture.profile, &fixture.cache_dir, &github, at(NOW))
            .expect("cached token is reused");

        assert_eq!(token.token, "cached-token");
    }

    #[test]
    fn keeps_separate_token_files_per_installation() {
        let fixture = fixture();
        let other = test_support::profile(&fixture.dir, 1, 3);
        seed(&fixture.token_file(), 1, 2, "2026-09-24T13:00:00Z");
        let server = issued("2026-09-24T13:00:00Z");

        let token = obtain(
            &other,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            at(NOW),
        )
        .expect("token is requested for the other installation");

        assert_eq!(token.token, "new-token");
        assert_eq!(token.installation_id, id(3));
        assert_eq!(fixture.stored().token, "cached-token");
        let other_file = read_toml::<CachedToken>(&fixture.cache_dir.join("token-1-3.toml"))
            .expect("cache is readable")
            .expect("cache holds a token");
        assert_eq!(other_file.token, "new-token");
    }

    #[test]
    fn refreshes_token_inside_the_refresh_margin() {
        let fixture = fixture();
        seed(&fixture.token_file(), 1, 2, "2026-09-24T12:04:59Z");
        let server = issued("2026-09-24T13:00:00Z");

        let token = obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            at(NOW),
        )
        .expect("token is refreshed");

        assert_eq!(token.token, "new-token");
        assert_eq!(server.requests().len(), 1);
        let stored = fixture.stored();
        assert_eq!(stored.token, "new-token");
        assert_eq!(stored.expires_at, at("2026-09-24T13:00:00Z"));
    }

    #[test]
    fn refreshes_token_whose_contents_name_another_installation() {
        let fixture = fixture();
        seed(&fixture.token_file(), 1, 3, "2026-09-24T13:00:00Z");
        let server = issued("2026-09-24T13:00:00Z");

        let token = obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            at(NOW),
        )
        .expect("token is refreshed");

        assert_eq!(token.token, "new-token");
        assert_eq!(token.installation_id, id(2));
    }

    #[test]
    fn replaces_unparsable_cache() {
        for contents in [b"not toml [".as_slice(), b"\xff\xfe not UTF-8"] {
            let fixture = fixture();
            fs_err::create_dir_all(&fixture.cache_dir).expect("cache directory is created");
            fs_err::write(fixture.token_file(), contents).expect("cache file is written");
            let server = issued("2026-09-24T13:00:00Z");

            let token = obtain(
                &fixture.profile,
                &fixture.cache_dir,
                &GitHub::with_api_url(&server.url),
                at(NOW),
            )
            .expect("token is refreshed");

            assert_eq!(token.token, "new-token", "{contents:?}");
            assert_eq!(fixture.stored().token, "new-token", "{contents:?}");
        }
    }

    #[test]
    fn keeps_existing_cache_when_the_request_fails() {
        let fixture = fixture();
        seed(&fixture.token_file(), 1, 2, "2026-09-24T11:00:00Z");
        let server = test_support::serve(vec![Response::json(
            401,
            r#"{"message":"Bad credentials"}"#,
        )]);

        let result = obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            at(NOW),
        );

        assert!(
            result.is_err(),
            "{:?}",
            result.map(|token| token.expires_at)
        );
        assert_eq!(fixture.stored().token, "cached-token");
    }

    #[test]
    fn describes_cache_states() {
        let fixture = fixture();
        let token = |installation_id, expires_at| CachedToken {
            app_id: id(1),
            installation_id: id(installation_id),
            token: "cached-token".to_owned(),
            expires_at: at(expires_at),
        };
        let describe =
            |cached: Option<CachedToken>| describe(cached.as_ref(), &fixture.profile, at(NOW));

        assert_eq!(describe(None), "none cached");
        assert_eq!(
            describe(Some(token(3, "2026-09-24T13:00:00Z"))),
            "cached for a different App or installation"
        );
        assert_eq!(
            describe(Some(token(2, "2026-09-24T11:00:00Z"))),
            "expired at 2026-09-24T11:00:00Z"
        );
        assert_eq!(
            describe(Some(token(2, "2026-09-24T12:03:00Z"))),
            "expires at 2026-09-24T12:03:00Z; the next gh command refreshes it"
        );
        assert_eq!(
            describe(Some(token(2, "2026-09-24T13:00:00Z"))),
            "valid until 2026-09-24T13:00:00Z"
        );
    }

    #[test]
    fn removes_the_temporary_file_after_writing() {
        let fixture = fixture();
        let server = issued("2026-09-24T13:00:00Z");
        obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            at(NOW),
        )
        .expect("token is refreshed");

        let names: Vec<String> = fs_err::read_dir(&fixture.cache_dir)
            .expect("cache directory is readable")
            .map(|entry| {
                entry
                    .expect("cache entry is readable")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["token-1-2.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn creates_cache_with_owner_only_modes() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = fixture();
        let server = issued("2026-09-24T13:00:00Z");
        obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            at(NOW),
        )
        .expect("token is refreshed");

        let mode = |path: &Utf8Path| {
            fs_err::metadata(path)
                .expect("path exists")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode(&fixture.cache_dir), 0o700);
        assert_eq!(mode(&fixture.token_file()), 0o600);
    }
}

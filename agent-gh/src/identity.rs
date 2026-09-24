use crate::cache;
use crate::config::Profile;
use crate::github::AppSlug;
use crate::github::GitHub;
use anyhow::Context;
use anyhow::Result;
use camino::Utf8Path;
use camino::Utf8PathBuf;
use jiff::Timestamp;
use serde::Deserialize;
use serde::Serialize;
use std::num::NonZeroU64;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CachedIdentity {
    pub(crate) app_id: NonZeroU64,
    pub(crate) slug: AppSlug,
    pub(crate) bot_user_id: NonZeroU64,
}

impl CachedIdentity {
    pub(crate) fn co_author(&self) -> String {
        let login = format!("{}[bot]", self.slug);
        format!(
            "{login} <{}+{login}@users.noreply.github.com>",
            self.bot_user_id
        )
    }
}

pub(crate) fn path(cache_dir: &Utf8Path, profile: &Profile) -> Utf8PathBuf {
    cache_dir.join(format!("identity-{}.toml", profile.app_id))
}

pub(crate) fn obtain(
    profile: &Profile,
    cache_dir: &Utf8Path,
    github: &GitHub,
    now: Timestamp,
) -> Result<CachedIdentity> {
    match read(cache_dir, profile)? {
        Some(cached) => Ok(cached),
        None => refresh(profile, cache_dir, github, now),
    }
}

pub(crate) fn refresh(
    profile: &Profile,
    cache_dir: &Utf8Path,
    github: &GitHub,
    now: Timestamp,
) -> Result<CachedIdentity> {
    let app = github
        .get_app(profile, now)
        .context("looking up the App slug")?;
    let token = cache::obtain(profile, cache_dir, github, now)?;
    let user = github
        .get_bot_user(&app.slug, &token.token)
        .with_context(|| format!("looking up the user ID of {}[bot]", app.slug))?;
    let identity = CachedIdentity {
        app_id: profile.app_id,
        slug: app.slug,
        bot_user_id: user.id,
    };
    cache::write_toml(&path(cache_dir, profile), &identity)
        .context("writing the identity cache")?;
    Ok(identity)
}

pub(crate) fn read(cache_dir: &Utf8Path, profile: &Profile) -> Result<Option<CachedIdentity>> {
    Ok(
        cache::read_toml::<CachedIdentity>(&path(cache_dir, profile))?
            .filter(|cached| cached.app_id == profile.app_id),
    )
}

pub(crate) fn describe(cached: Option<&CachedIdentity>) -> String {
    cached.map_or_else(|| "not cached".to_owned(), CachedIdentity::co_author)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use crate::test_support::Response;
    use jiff::SignedDuration;
    use tempfile::TempDir;

    const NOW: &str = "2026-09-24T12:00:00Z";
    const UNREACHABLE_API: &str = "http://127.0.0.1:9";
    const APP: &str = r#"{"id":1,"slug":"test-app"}"#;
    const USER: &str = r#"{"login":"test-app[bot]","id":332833177}"#;
    const CO_AUTHOR: &str = "test-app[bot] <332833177+test-app[bot]@users.noreply.github.com>";

    struct Fixture {
        _dir: TempDir,
        profile: Profile,
        cache_dir: Utf8PathBuf,
    }

    impl Fixture {
        fn identity_file(&self) -> Utf8PathBuf {
            self.cache_dir.join("identity-1.toml")
        }

        fn seed_identity(&self, contents: &str) {
            fs_err::create_dir_all(&self.cache_dir).expect("cache directory is created");
            fs_err::write(self.identity_file(), contents).expect("identity cache is written");
        }

        fn seed_token(&self) {
            let expires_at = now()
                .checked_add(SignedDuration::from_secs(60 * 60))
                .expect("expiry is representable");
            fs_err::create_dir_all(&self.cache_dir).expect("cache directory is created");
            fs_err::write(
                self.cache_dir.join("token-1-2.toml"),
                format!(
                    "app_id = 1\ninstallation_id = 2\ntoken = \"cached-token\"\nexpires_at = \"{expires_at}\"\n"
                ),
            )
            .expect("token cache is written");
        }

        fn stored(&self) -> String {
            fs_err::read_to_string(self.identity_file()).expect("identity cache is readable")
        }
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("temporary directory is created");
        let root =
            Utf8PathBuf::try_from(dir.path().to_path_buf()).expect("temporary directory is UTF-8");
        Fixture {
            profile: test_support::profile(&dir, 1, 2),
            cache_dir: root.join("cache"),
            _dir: dir,
        }
    }

    fn now() -> Timestamp {
        NOW.parse().expect("fixture timestamp is valid")
    }

    fn identity(app_id: u64) -> String {
        format!("app_id = {app_id}\nslug = \"cached-app\"\nbot_user_id = 7\n")
    }

    #[test]
    fn formats_co_author_with_bot_user_id() {
        let identity = CachedIdentity {
            app_id: NonZeroU64::new(5_043_706).expect("fixture ID is nonzero"),
            slug: AppSlug::try_from("kvnxiao-agent".to_owned()).expect("fixture slug is valid"),
            bot_user_id: NonZeroU64::new(332_833_177).expect("fixture ID is nonzero"),
        };
        assert_eq!(
            identity.co_author(),
            "kvnxiao-agent[bot] <332833177+kvnxiao-agent[bot]@users.noreply.github.com>"
        );
    }

    #[test]
    fn reuses_matching_identity_without_network_requests() {
        let fixture = fixture();
        fixture.seed_identity(&identity(1));

        let identity = obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(UNREACHABLE_API),
            now(),
        )
        .expect("cached identity is reused");

        assert_eq!(
            identity.co_author(),
            "cached-app[bot] <7+cached-app[bot]@users.noreply.github.com>"
        );
    }

    #[test]
    fn fetches_identity_when_the_cache_is_unusable() {
        let cases = [
            identity(3),
            "not toml [".to_owned(),
            "app_id = 1\nslug = \"bad\\nslug\"\nbot_user_id = 7\n".to_owned(),
        ];
        for contents in cases {
            let fixture = fixture();
            fixture.seed_identity(&contents);
            fixture.seed_token();
            let server =
                test_support::serve(vec![Response::json(200, APP), Response::json(200, USER)]);

            let identity = obtain(
                &fixture.profile,
                &fixture.cache_dir,
                &GitHub::with_api_url(&server.url),
                now(),
            )
            .expect("identity is fetched");

            assert_eq!(identity.co_author(), CO_AUTHOR, "{contents:?}");
            let requests = server.requests();
            let paths: Vec<&str> = requests
                .iter()
                .map(|request| request.path.as_str())
                .collect();
            assert_eq!(paths, ["/app", "/users/test-app%5Bbot%5D"], "{contents:?}");
            assert_eq!(
                fixture.stored(),
                "app_id = 1\nslug = \"test-app\"\nbot_user_id = 332833177\n",
                "{contents:?}"
            );
        }
    }

    #[test]
    fn requests_installation_token_for_user_lookup_on_token_cache_miss() {
        let fixture = fixture();
        let server = test_support::serve(vec![
            Response::json(200, APP),
            Response::json(
                201,
                r#"{"token":"new-token","expires_at":"2026-09-24T13:00:00Z"}"#,
            ),
            Response::json(200, USER),
        ]);

        let identity = obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            now(),
        )
        .expect("identity is fetched");

        assert_eq!(identity.co_author(), CO_AUTHOR);
        let requests = server.requests();
        let paths: Vec<&str> = requests
            .iter()
            .map(|request| request.path.as_str())
            .collect();
        assert_eq!(
            paths,
            [
                "/app",
                "/app/installations/2/access_tokens",
                "/users/test-app%5Bbot%5D"
            ]
        );
        assert_eq!(requests[2].header("authorization"), "Bearer new-token");
    }

    #[test]
    fn keeps_existing_identity_when_the_fetch_fails() {
        let fixture = fixture();
        fixture.seed_identity(&identity(3));
        let server = test_support::serve(vec![Response::json(
            401,
            r#"{"message":"Bad credentials"}"#,
        )]);

        let result = obtain(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            now(),
        );

        let Err(error) = result else {
            panic!("fetch should fail");
        };
        let message = format!("{error:#}");
        assert!(
            message.starts_with("looking up the App slug: GitHub returned HTTP 401"),
            "{message}"
        );
        assert_eq!(fixture.stored(), identity(3));
    }

    #[test]
    fn refresh_replaces_matching_identity() {
        let fixture = fixture();
        fixture.seed_identity(&identity(1));
        fixture.seed_token();
        let server = test_support::serve(vec![Response::json(200, APP), Response::json(200, USER)]);

        let identity = refresh(
            &fixture.profile,
            &fixture.cache_dir,
            &GitHub::with_api_url(&server.url),
            now(),
        )
        .expect("identity is refreshed");

        assert_eq!(identity.co_author(), CO_AUTHOR);
        assert!(
            fixture.stored().contains("test-app"),
            "{}",
            fixture.stored()
        );
    }

    #[test]
    fn describes_cached_and_missing_identity() {
        let fixture = fixture();
        assert_eq!(describe(None), "not cached");
        fixture.seed_identity(&identity(1));
        let cached = read(&fixture.cache_dir, &fixture.profile).expect("cache is readable");
        assert_eq!(
            describe(cached.as_ref()),
            "cached-app[bot] <7+cached-app[bot]@users.noreply.github.com>"
        );
        fixture.seed_identity(&identity(3));
        let cached = read(&fixture.cache_dir, &fixture.profile).expect("cache is readable");
        assert_eq!(describe(cached.as_ref()), "not cached");
    }
}

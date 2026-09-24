use crate::config::Profile;
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use camino::Utf8Path;
use jiff::SignedDuration;
use jiff::Timestamp;
use jsonwebtoken::Algorithm;
use jsonwebtoken::EncodingKey;
use jsonwebtoken::Header;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Duration;
use ureq::Agent;
use ureq::Body;
use ureq::RequestBuilder;
use ureq::http::Response;
use ureq::http::StatusCode;
use ureq::tls::TlsConfig;

const API_URL: &str = "https://api.github.com";
const API_VERSION: &str = "2022-11-28";
const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const JWT_BACKDATE: SignedDuration = SignedDuration::from_secs(60);
const JWT_LIFETIME: SignedDuration = SignedDuration::from_secs(9 * 60);

pub(crate) struct GitHub {
    agent: Agent,
    api_url: String,
}

#[derive(Deserialize)]
pub(crate) struct InstallationToken {
    pub(crate) token: String,
    pub(crate) expires_at: Timestamp,
}

#[derive(Serialize)]
struct Claims {
    iat: i64,
    exp: i64,
    iss: u64,
}

#[derive(Deserialize)]
pub(crate) struct App {
    pub(crate) id: NonZeroU64,
    pub(crate) slug: AppSlug,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct AppSlug(String);

#[derive(Deserialize)]
pub(crate) struct User {
    pub(crate) id: NonZeroU64,
}

#[derive(Deserialize)]
struct ErrorResponse {
    message: String,
}

impl GitHub {
    pub(crate) fn new() -> Self {
        Self::with_api_url(API_URL)
    }

    pub(crate) fn with_api_url(api_url: &str) -> Self {
        let tls = TlsConfig::builder()
            .unversioned_rustls_crypto_provider(Arc::new(
                rustls::crypto::aws_lc_rs::default_provider(),
            ))
            .build();
        let config = Agent::config_builder()
            .tls_config(tls)
            .timeout_global(Some(REQUEST_TIMEOUT))
            .max_redirects(0)
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .build();
        Self {
            agent: Agent::new_with_config(config),
            api_url: api_url.to_owned(),
        }
    }

    pub(crate) fn create_installation_token(
        &self,
        profile: &Profile,
        now: Timestamp,
    ) -> Result<InstallationToken> {
        let jwt = profile_jwt(profile, now)?;
        let url = format!(
            "{}/app/installations/{}/access_tokens",
            self.api_url, profile.installation_id
        );
        let response = authorize(self.agent.post(&url), &jwt)
            .send_empty()
            .context("requesting an installation token from GitHub")?;
        parse_token_response(&read_body(
            response,
            StatusCode::CREATED,
            "installation token",
        )?)
    }

    pub(crate) fn get_app(&self, profile: &Profile, now: Timestamp) -> Result<App> {
        let jwt = profile_jwt(profile, now)?;
        let url = format!("{}/app", self.api_url);
        let response = authorize(self.agent.get(&url), &jwt)
            .call()
            .context("requesting the App from GitHub")?;
        let body = read_body(response, StatusCode::OK, "App")?;
        let app: App = serde_json::from_str(&body).context("parsing the App response")?;
        if app.id != profile.app_id {
            bail!(
                "GitHub returned App ID {} for app_id {}",
                app.id,
                profile.app_id
            );
        }
        Ok(app)
    }

    pub(crate) fn get_bot_user(&self, slug: &AppSlug, token: &str) -> Result<User> {
        let url = format!("{}/users/{slug}%5Bbot%5D", self.api_url);
        let response = authorize(self.agent.get(&url), token)
            .call()
            .context("requesting the bot user from GitHub")?;
        let body = read_body(response, StatusCode::OK, "user")?;
        serde_json::from_str(&body).context("parsing the user response")
    }
}

impl TryFrom<String> for AppSlug {
    type Error = anyhow::Error;

    fn try_from(slug: String) -> Result<Self> {
        let unreserved = |byte: u8| byte.is_ascii_alphanumeric() || b"-._~".contains(&byte);
        if slug.is_empty() || !slug.bytes().all(unreserved) {
            bail!("the App slug {slug:?} is not a valid URL path segment");
        }
        Ok(Self(slug))
    }
}

impl From<AppSlug> for String {
    fn from(slug: AppSlug) -> Self {
        slug.0
    }
}

impl fmt::Display for AppSlug {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn authorize<B>(request: RequestBuilder<B>, bearer: &str) -> RequestBuilder<B> {
    request
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", API_VERSION)
        .header("Authorization", format!("Bearer {bearer}"))
}

fn read_body(mut response: Response<Body>, expected: StatusCode, name: &str) -> Result<String> {
    let status = response.status();
    let body = response.body_mut().read_to_string();
    if status != expected {
        return Err(api_error(status, body.as_deref().unwrap_or_default()));
    }
    body.with_context(|| format!("reading the {name} response"))
}

fn profile_jwt(profile: &Profile, now: Timestamp) -> Result<String> {
    let key = load_signing_key(&profile.private_key_path)?;
    app_jwt(profile.app_id, &key, now)
}

fn load_signing_key(path: &Utf8Path) -> Result<EncodingKey> {
    let pem = fs_err::read(path)?;
    EncodingKey::from_rsa_pem(&pem).with_context(|| format!("parsing private key {path}"))
}

fn app_jwt(app_id: NonZeroU64, key: &EncodingKey, now: Timestamp) -> Result<String> {
    let claims = Claims {
        iat: now.checked_sub(JWT_BACKDATE)?.as_second(),
        exp: now.checked_add(JWT_LIFETIME)?.as_second(),
        iss: app_id.get(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key)
        .context("signing the App JWT")
}

fn parse_token_response(body: &str) -> Result<InstallationToken> {
    let token: InstallationToken =
        serde_json::from_str(body).context("parsing the installation token response")?;
    if token.token.is_empty() {
        bail!("GitHub returned an empty installation token");
    }
    Ok(token)
}

fn api_error(status: StatusCode, body: &str) -> anyhow::Error {
    let code = status.as_u16();
    let hint = match code {
        300..=399 => " (redirects are not followed)",
        401 => " (check app_id, the private key, and the system clock)",
        404 => " (check app_id, installation_id, and that the App is installed)",
        _ => "",
    };
    match serde_json::from_str::<ErrorResponse>(body) {
        Ok(error) => anyhow!("GitHub returned HTTP {code}{hint}: {}", error.message),
        Err(_) => anyhow!("GitHub returned HTTP {code}{hint}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use crate::test_support::Response;
    use camino::Utf8PathBuf;
    use jsonwebtoken::DecodingKey;
    use jsonwebtoken::Validation;

    const NOW: &str = "2026-09-24T12:00:00Z";

    fn now() -> Timestamp {
        NOW.parse().expect("fixture timestamp is valid")
    }

    fn profile(key_path: Utf8PathBuf) -> Profile {
        Profile {
            name: "test".to_owned(),
            app_id: NonZeroU64::new(1234).expect("fixture ID is nonzero"),
            installation_id: NonZeroU64::new(5678).expect("fixture ID is nonzero"),
            private_key_path: key_path,
            run_as_user: Vec::new(),
        }
    }

    fn only(requests: &[test_support::Request]) -> &test_support::Request {
        let [request] = requests else {
            panic!("expected one request, got {}", requests.len());
        };
        request
    }

    fn slug() -> AppSlug {
        AppSlug::try_from("test-app".to_owned()).expect("fixture slug is valid")
    }

    fn app_error(response: Response) -> String {
        let dir = tempfile::tempdir().expect("temporary directory is created");
        let server = test_support::serve(vec![response]);
        match GitHub::with_api_url(&server.url)
            .get_app(&profile(test_support::write_key(&dir)), now())
        {
            Ok(app) => panic!("App {} should be rejected", app.slug),
            Err(error) => format!("{error:#}"),
        }
    }

    fn request_error(response: Response) -> (String, Vec<test_support::Request>) {
        let dir = tempfile::tempdir().expect("temporary directory is created");
        let server = test_support::serve(vec![response]);
        let github = GitHub::with_api_url(&server.url);
        let result =
            github.create_installation_token(&profile(test_support::write_key(&dir)), now());
        let requests = server.requests();
        match result {
            Ok(_) => panic!("token request should fail"),
            Err(error) => (format!("{error:#}"), requests),
        }
    }

    #[test]
    fn signs_rs256_jwt_with_backdated_issue_time() {
        let key = EncodingKey::from_rsa_pem(test_support::key_pem().as_bytes())
            .expect("generated key parses");
        let jwt =
            app_jwt(NonZeroU64::new(1234).expect("nonzero"), &key, now()).expect("JWT is signed");

        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        let decoded = jsonwebtoken::decode::<serde_json::Value>(
            &jwt,
            &DecodingKey::from_rsa_der(test_support::public_key_der()),
            &validation,
        )
        .expect("signature verifies with the generated public key");

        let now = now().as_second();
        assert_eq!(decoded.header.alg, Algorithm::RS256);
        assert_eq!(decoded.claims["iss"], 1234);
        assert_eq!(decoded.claims["iat"], now - 60);
        assert_eq!(decoded.claims["exp"], now + 540);
    }

    #[test]
    fn requests_installation_token_with_app_jwt() {
        let dir = tempfile::tempdir().expect("temporary directory is created");
        let server = test_support::serve(vec![Response::json(
            201,
            r#"{"token":"issued-token","expires_at":"2026-09-24T13:00:00Z","permissions":{}}"#,
        )]);
        let github = GitHub::with_api_url(&server.url);

        let token = github
            .create_installation_token(&profile(test_support::write_key(&dir)), now())
            .expect("token request succeeds");

        assert_eq!(token.token, "issued-token");
        assert_eq!(token.expires_at.to_string(), "2026-09-24T13:00:00Z");
        let requests = server.requests();
        let request = only(&requests);
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/app/installations/5678/access_tokens");
        let authorization = request.header("authorization");
        assert!(authorization.starts_with("Bearer ey"), "{authorization}");
        assert_eq!(request.header("accept"), "application/vnd.github+json");
        assert_eq!(request.header("x-github-api-version"), "2022-11-28");
        let user_agent = request.header("user-agent");
        assert!(user_agent.starts_with("agent-gh/"), "{user_agent}");
    }

    #[test]
    fn reports_status_and_message_without_the_jwt() {
        let (message, requests) = request_error(Response::json(
            401,
            r#"{"message":"A JSON web token could not be decoded","documentation_url":"https://docs.github.com"}"#,
        ));
        assert!(message.contains("HTTP 401"), "{message}");
        assert!(message.contains("system clock"), "{message}");
        assert!(
            message.contains("A JSON web token could not be decoded"),
            "{message}"
        );
        let jwt = only(&requests)
            .header("authorization")
            .trim_start_matches("Bearer ");
        assert!(!message.contains(jwt), "{message}");
    }

    #[test]
    fn omits_unparsed_error_bodies() {
        let (message, _) = request_error(Response::json(500, "internal detail"));
        assert_eq!(message, "GitHub returned HTTP 500");
    }

    #[test]
    fn reports_status_when_the_error_body_is_not_utf8() {
        let (message, _) = request_error(Response::json(502, b"\xff\xfe proxy error"));
        assert_eq!(message, "GitHub returned HTTP 502");
    }

    #[test]
    fn does_not_follow_redirects() {
        let target = test_support::serve(Vec::new());
        let (message, _) = request_error(Response::redirect(&target.url));
        assert!(message.contains("HTTP 302"), "{message}");
        assert!(message.contains("redirects are not followed"), "{message}");
        assert_eq!(target.requests().len(), 0);
    }

    #[test]
    fn rejects_empty_token() {
        let (message, _) = request_error(Response::json(
            201,
            r#"{"token":"","expires_at":"2026-09-24T13:00:00Z"}"#,
        ));
        assert!(message.contains("empty installation token"), "{message}");
    }

    #[test]
    fn rejects_malformed_expiry() {
        let (message, _) = request_error(Response::json(
            201,
            r#"{"token":"issued-token","expires_at":"soon"}"#,
        ));
        assert!(
            message.contains("parsing the installation token response"),
            "{message}"
        );
        assert!(!message.contains("issued-token"), "{message}");
    }

    #[test]
    fn reports_unreadable_private_key() {
        let dir = tempfile::tempdir().expect("temporary directory is created");
        let path = Utf8PathBuf::try_from(dir.path().join("missing.pem"))
            .expect("temporary directory is UTF-8");
        let github = GitHub::with_api_url("http://127.0.0.1:9");
        let Err(error) = github.create_installation_token(&profile(path.clone()), now()) else {
            panic!("missing key should fail");
        };
        assert!(format!("{error:#}").contains(path.as_str()), "{error:#}");
    }

    #[test]
    fn requests_app_with_app_jwt() {
        let dir = tempfile::tempdir().expect("temporary directory is created");
        let server = test_support::serve(vec![Response::json(
            200,
            r#"{"id":1234,"slug":"test-app","name":"Test App"}"#,
        )]);

        let app = GitHub::with_api_url(&server.url)
            .get_app(&profile(test_support::write_key(&dir)), now())
            .expect("App request succeeds");

        assert_eq!(app.slug.to_string(), "test-app");
        let requests = server.requests();
        let request = only(&requests);
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/app");
        let authorization = request.header("authorization");
        assert!(authorization.starts_with("Bearer ey"), "{authorization}");
        assert_eq!(request.header("accept"), "application/vnd.github+json");
        assert_eq!(request.header("x-github-api-version"), "2022-11-28");
    }

    #[test]
    fn reports_app_errors() {
        let cases = [
            (
                Response::json(200, r#"{"id":99,"slug":"other-app"}"#),
                "GitHub returned App ID 99 for app_id 1234",
            ),
            (
                Response::json(
                    401,
                    r#"{"message":"A JSON web token could not be decoded"}"#,
                ),
                "GitHub returned HTTP 401 (check app_id, the private key, and the system clock): A JSON web token could not be decoded",
            ),
        ];
        for (response, expected) in cases {
            assert_eq!(app_error(response), expected);
        }
    }

    #[test]
    fn rejects_app_slug_outside_url_path_characters() {
        for slug in ["", "test/app", "test app", r"test\napp", "test%5Bapp"] {
            let message = app_error(Response::json(
                200,
                format!(r#"{{"id":1234,"slug":"{slug}"}}"#),
            ));
            assert!(
                message.contains("not a valid URL path segment"),
                "{slug:?}: {message}"
            );
        }
    }

    #[test]
    fn requests_bot_user_with_installation_token() {
        let server = test_support::serve(vec![Response::json(
            200,
            r#"{"login":"test-app[bot]","id":332833177,"type":"Bot"}"#,
        )]);

        let user = GitHub::with_api_url(&server.url)
            .get_bot_user(&slug(), "installation-token")
            .expect("user request succeeds");

        assert_eq!(user.id.get(), 332_833_177);
        let requests = server.requests();
        let request = only(&requests);
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/users/test-app%5Bbot%5D");
        assert_eq!(request.header("authorization"), "Bearer installation-token");
        assert_eq!(request.header("accept"), "application/vnd.github+json");
        assert_eq!(request.header("x-github-api-version"), "2022-11-28");
    }

    #[test]
    fn reports_bot_user_request_errors() {
        let server = test_support::serve(vec![Response::json(404, r#"{"message":"Not Found"}"#)]);
        let Err(error) =
            GitHub::with_api_url(&server.url).get_bot_user(&slug(), "installation-token")
        else {
            panic!("missing user should fail");
        };
        let message = format!("{error:#}");
        assert!(message.starts_with("GitHub returned HTTP 404"), "{message}");
        assert!(message.ends_with(": Not Found"), "{message}");
    }
}

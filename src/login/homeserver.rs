//! Resolve sign-in destinations before sending any credentials.
use anyhow::{bail, Result};
use matrix_sdk::{
    config::RequestConfig,
    ruma::{
        api::client::{session::get_login_types::v3::LoginType, uiaa::UserIdentifier},
        UserId,
    },
    Client,
};
use url::Url;

/// An explicit server wins; otherwise discover from a full Matrix ID. A local
/// username uses matrix.org. Email domains are never treated as homeservers.
pub fn login_server(user: &str, server: Option<&str>) -> Result<String> {
    let user = user.trim();
    let server = server.map(str::trim).filter(|s| !s.is_empty());
    if let Some(server) = server {
        if server.chars().any(char::is_whitespace) || server.contains('\\') {
            bail!("Enter a server name or an HTTP/HTTPS homeserver URL.");
        }
        if server.contains("://") {
            let url = Url::parse(server)?;
            if !matches!(url.scheme(), "https" | "http")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                bail!("Use an HTTP/HTTPS homeserver URL without credentials, query, or fragment.");
            }
            return Ok(url.to_string());
        }
        let name = matrix_sdk::ruma::ServerName::parse(server)
            .map_err(|_| anyhow::anyhow!("Enter a valid server name, such as matrix.org."))?;
        return Ok(name.to_string());
    }
    if user.starts_with('@') && user.contains(':') {
        let id = UserId::parse(user)
            .map_err(|_| anyhow::anyhow!("Use a Matrix ID like @name:example.org."))?;
        return Ok(id.server_name().to_string());
    }
    if !user.starts_with('@') && user.contains('@') {
        bail!("Enter your homeserver when signing in with an email address.");
    }
    Ok("matrix.org".to_owned())
}

pub fn password_identifier(user: &str) -> Result<UserIdentifier> {
    let user = user.trim();
    if user.is_empty() || user.chars().any(char::is_whitespace) {
        bail!("Enter your Matrix ID, username, or registered email address.");
    }
    if !user.starts_with('@') && user.contains('@') {
        return Ok(UserIdentifier::Email(
            matrix_sdk::ruma::api::client::uiaa::EmailUserIdentifier::new(user.to_owned()),
        ));
    }
    if user.contains(':') {
        UserId::parse(user)
            .map_err(|_| anyhow::anyhow!("Use a Matrix ID like @name:example.org."))?;
    }
    Ok(UserIdentifier::Matrix(
        matrix_sdk::ruma::api::client::uiaa::MatrixUserIdentifier::new(
            if user.contains(':') {
                user
            } else {
                user.trim_start_matches('@')
            }
            .to_owned(),
        ),
    ))
}

#[derive(Clone, Debug)]
pub struct LoginProvider {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct LoginMethods {
    pub homeserver: String,
    pub password: bool,
    pub sso: bool,
    pub oauth_aware_preferred: bool,
    pub browser_registration: bool,
    pub providers: Vec<LoginProvider>,
}

pub async fn login_methods(client: &Client) -> Result<LoginMethods> {
    let response = client.matrix_auth().get_login_types().await?;
    let mut methods = LoginMethods {
        homeserver: client.homeserver().to_string(),
        password: false,
        sso: false,
        oauth_aware_preferred: false,
        browser_registration: false,
        providers: Vec::new(),
    };
    for flow in response.flows {
        match flow {
            LoginType::Password(_) => methods.password = true,
            LoginType::Sso(sso) => {
                methods.sso = true;
                methods.oauth_aware_preferred = sso.oauth_aware_preferred;
                if sso.oauth_aware_preferred {
                    methods.browser_registration = browser_registration_supported(client).await;
                }
                methods
                    .providers
                    .extend(sso.identity_providers.into_iter().map(|p| LoginProvider {
                        id: p.id,
                        name: p.name,
                    }));
            }
            _ => {}
        }
    }
    Ok(methods)
}

/// Registration is a user action, not the OAuth dynamic client registration
/// endpoint. The issuer advertises browser account creation with `prompt=create`.
async fn browser_registration_supported(client: &Client) -> bool {
    let Ok(metadata_url) = client.homeserver().join("_matrix/client/v1/auth_metadata") else {
        return false;
    };
    let Ok(response) = matrix_sdk::reqwest::Client::new().get(metadata_url).timeout(std::time::Duration::from_secs(10)).send().await else {
        return false;
    };
    if !response.status().is_success() {
        return false;
    }
    let Ok(body) = response.bytes().await else {
        return false;
    };
    let Ok(metadata) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return false;
    };
    metadata["prompt_values_supported"].as_array().is_some_and(|values| values.iter().any(|value| value == "create"))
}

/// Memory-only discovery: checking a server does not create a session/database.
pub async fn discover(user: &str, server: &str) -> Result<LoginMethods> {
    let destination = login_server(user, Some(server))?;
    let builder = Client::builder()
        .server_name_or_homeserver_url(destination)
        .request_config(
            RequestConfig::new()
                .timeout(std::time::Duration::from_secs(15))
                .retry_limit(0),
        );
    let client = crate::sliding_sync::use_android_tls_roots(builder)
        .build()
        .await?;
    login_methods(&client).await
}

/// Complete a legacy registration when the homeserver offers a simple UIAA
/// dummy or registration-token stage. Other stages need their own client UI.
pub async fn register_account(client: &Client, username: String, password: String, token: String) -> Result<()> {
    use matrix_sdk::ruma::api::client::{account::register::v3::Request, uiaa::{AuthData, AuthType, Dummy, RegistrationToken}};

    let mut request = Request::new();
    request.username = Some(username);
    request.password = Some(password);
    request.refresh_token = true;
    match client.matrix_auth().register(request.clone()).await {
        Ok(_) => {}
        Err(error) => {
            let Some(challenge) = error.as_uiaa_response() else { return Err(error.into()) };
            let session_id = challenge.session.clone();
            let flow = challenge.flows.iter().find(|flow| flow.stages.as_slice() == [AuthType::Dummy])
                .or_else(|| challenge.flows.iter().find(|flow| flow.stages.as_slice() == [AuthType::RegistrationToken]));
            match flow.map(|flow| flow.stages.first()) {
                Some(Some(AuthType::Dummy)) => {
                    let mut auth = Dummy::new();
                    auth.session = session_id;
                    request.auth = Some(AuthData::Dummy(auth));
                }
                Some(Some(AuthType::RegistrationToken)) => {
                    if token.is_empty() {
                        bail!("This server requires a registration token. Enter it and try again.");
                    }
                    let mut auth = RegistrationToken::new(token);
                    auth.session = session_id;
                    request.auth = Some(AuthData::RegistrationToken(auth));
                }
                _ => bail!("This server requires a registration step that Rinx cannot complete yet."),
            }
            client.matrix_auth().register(request).await?;
        }
    }
    if !client.matrix_auth().logged_in() {
        bail!("Account created, but the server did not issue a login session. Please sign in.");
    }
    Ok(())
}

#[cfg(not(target_os = "ios"))]
pub async fn browser_login<F, Fut>(client: &Client, provider_id: Option<&str>, register: bool, open: F) -> matrix_sdk::Result<()>
where
    F: FnOnce(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = matrix_sdk::Result<()>> + Send + 'static,
{
    let mut login = client.matrix_auth().login_sso(move |sso_url| async move {
        let sso_url = if register {
            let mut url = url::Url::parse(&sso_url).map_err(|error| matrix_sdk::Error::Io(std::io::Error::other(error)))?;
            url.query_pairs_mut().append_pair("action", "register");
            url.into()
        } else {
            sso_url
        };
        open(sso_url).await
    });
    if let Some(id) = provider_id {
        login = login.identity_provider_id(id);
    }
    login
        .initial_device_display_name("Rinx")
        .request_refresh_token()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_id_discovers_its_server_and_explicit_url_wins() {
        assert_eq!(
            login_server(" @alex:example.org:8448 ", None).unwrap(),
            "example.org:8448"
        );
        assert_eq!(
            login_server("@alex:example.org", Some(" http://127.0.0.1:18120/ ")).unwrap(),
            "http://127.0.0.1:18120/"
        );
        assert_eq!(login_server("alex", None).unwrap(), "matrix.org");
        assert!(login_server("alex@example.org", None).is_err());
    }
    #[test]
    fn malformed_destinations_are_rejected_before_credentials() {
        for server in [
            "javascript://host",
            "https://user:secret@example.org",
            "https://example.org?token=secret",
            "https://example.org/#login",
            "foo bar",
            "https://example.org\\evil",
        ] {
            assert!(login_server("alex", Some(server)).is_err(), "{server}");
        }
        assert!(login_server("@alex:", None).is_err());
    }
    #[test]
    fn identifiers_use_matrix_protocol_types() {
        assert_eq!(
            serde_json::to_value(password_identifier("@alex").unwrap()).unwrap(),
            serde_json::json!({"type":"m.id.user","user":"alex"})
        );
        assert_eq!(
            serde_json::to_value(password_identifier(" alex@example.org ").unwrap()).unwrap(),
            serde_json::json!({"type":"m.id.thirdparty","medium":"email","address":"alex@example.org"})
        );
        assert!(password_identifier(" ").is_err());
        assert!(password_identifier("@alex:example.org").is_ok());
    }

    /// A local Matrix protocol fixture; no production credentials or browser.
    struct Server {
        url: String,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        requests: std::sync::Arc<std::sync::Mutex<Vec<(String, serde_json::Value)>>>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn new() -> Self {
            Self::with_registration(None)
        }
        fn with_registration(registration_token: Option<&'static str>) -> Self {
            use std::{
                io::{Read, Write},
                sync::{
                    Arc, Mutex,
                    atomic::{AtomicBool, Ordering},
                },
            };
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let received = requests.clone();
            let thread = std::thread::spawn(move || {
                while !stopped.load(Ordering::Relaxed) {
                    let Ok((mut socket, _)) = listener.accept() else {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    let mut buf = [0; 4096];
                    let (head, body) = loop {
                        let count = socket.read(&mut buf).unwrap();
                        if count == 0 {
                            panic!("Incomplete fixture request");
                        }
                        bytes.extend_from_slice(&buf[..count]);
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
                            let len = head
                                .lines()
                                .find_map(|l| {
                                    l.to_ascii_lowercase()
                                        .strip_prefix("content-length:")
                                        .map(|n| n.trim().parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            if bytes.len() >= end + 4 + len {
                                break (head, bytes[end + 4..end + 4 + len].to_vec());
                            }
                        }
                    };
                    let route = head.lines().next().unwrap().to_owned();
                    let json = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                    received.lock().unwrap().push((route.clone(), json.clone()));
                    let mut status = "200 OK";
                    let response = if route.contains("/versions ") {
                        serde_json::json!({"versions":["v1.11"],"unstable_features":{}})
                    } else if route.contains("/auth_metadata ") {
                        serde_json::json!({"prompt_values_supported":["login","create"]})
                    } else if route.starts_with("GET ") && route.contains("/login ") {
                        serde_json::json!({"flows":[{"type":"m.login.password"},{"type":"m.login.sso","oauth_aware_preferred":true,"identity_providers":[{"id":"company-custom-id","name":"Company SSO"}]},{"type":"m.login.token"}]})
                    } else if route.starts_with("POST ") && route.contains("/register ") && registration_token.is_some() {
                        let stage = if registration_token == Some("") { "m.login.dummy" } else { "m.login.registration_token" };
                        let completed = json["auth"]["type"] == stage && (stage == "m.login.dummy" || json["auth"]["token"] == registration_token.unwrap());
                        if completed {
                            serde_json::json!({"user_id":"@fixture:localhost","device_id":"TESTDEVICE","access_token":"fixture-access","refresh_token":"fixture-refresh"})
                        } else {
                            status = "401 Unauthorized";
                            serde_json::json!({"flows":[{"stages":[stage]}],"session":"fixture-registration-session"})
                        }
                    } else if route.starts_with("POST ") && route.contains("/login ") {
                        serde_json::json!({"user_id":"@fixture:localhost","device_id":"TESTDEVICE","access_token":"fixture-access","refresh_token":"fixture-refresh"})
                    } else {
                        serde_json::json!({})
                    };
                    let body = response.to_string();
                    let _ = write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                }
            });
            Self {
                url,
                stop,
                requests,
                thread: Some(thread),
            }
        }
        async fn client(&self) -> Client {
            Client::builder()
                .homeserver_url(&self.url)
                .request_config(RequestConfig::new().retry_limit(0))
                .build()
                .await
                .unwrap()
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let result = self.thread.take().unwrap().join();
            if !std::thread::panicking() { result.unwrap(); }
        }
    }

    #[tokio::test]
    async fn reads_actual_advertised_methods_and_custom_provider_names() {
        let server = Server::new();
        let methods = login_methods(&server.client().await).await.unwrap();
        assert!(methods.password && methods.sso);
        assert!(methods.oauth_aware_preferred && methods.browser_registration);
        assert_eq!(methods.providers[0].id, "company-custom-id");
        assert_eq!(methods.providers[0].name, "Company SSO");
    }

    #[tokio::test]
    async fn legacy_registration_completes_dummy_challenge() {
        let server = Server::with_registration(Some(""));
        let client = server.client().await;
        register_account(&client, "new_user".into(), "a-strong-password".into(), "".into()).await.unwrap();
        assert!(client.matrix_auth().logged_in());
        let requests = server.requests.lock().unwrap();
        let registrations: Vec<_> = requests.iter().filter(|(route, _)| route.contains("/register ")).collect();
        assert_eq!(registrations.len(), 2);
        assert_eq!(registrations[1].1["auth"]["type"], "m.login.dummy");
        assert_eq!(registrations[1].1["auth"]["session"], "fixture-registration-session");
    }

    #[tokio::test]
    async fn token_registration_requires_a_token_before_retrying() {
        let server = Server::with_registration(Some("invite"));
        let client = server.client().await;
        let error = register_account(&client, "new_user".into(), "a-strong-password".into(), "".into()).await.unwrap_err();
        assert!(error.to_string().contains("registration token"));
        register_account(&client, "new_user".into(), "a-strong-password".into(), "invite".into()).await.unwrap();
        let requests = server.requests.lock().unwrap();
        let registrations: Vec<_> = requests.iter().filter(|(route, _)| route.contains("/register ")).collect();
        assert_eq!(registrations.len(), 3);
        assert_eq!(registrations[2].1["auth"]["token"], "invite");
    }

    #[cfg(not(target_os = "ios"))]
    #[tokio::test]
    async fn selected_provider_uses_advertised_id_and_exchanges_token() {
        let server = Server::new();
        let client = server.client().await;
        browser_login(&client, Some("company-custom-id"), false, |link| async move {
            let url = Url::parse(&link).unwrap();
            assert!(url.path().ends_with("/login/sso/redirect/company-custom-id"));
            let redirect = url
                .query_pairs()
                .find(|(k, _)| k == "redirectUrl")
                .unwrap()
                .1
                .into_owned();
            let mut callback = Url::parse(&redirect).unwrap();
            assert!(matches!(
                callback.host_str(),
                Some("127.0.0.1" | "[::1]" | "localhost")
            ));
            callback
                .query_pairs_mut()
                .append_pair("loginToken", "fixture-login-token");
            matrix_sdk::reqwest::get(callback)
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(client.user_id().unwrap().as_str(), "@fixture:localhost");
        let requests = server.requests.lock().unwrap();
        let logins: Vec<_> = requests
            .iter()
            .filter(|(route, _)| route.starts_with("POST ") && route.contains("/login "))
            .collect();
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].1["type"], "m.login.token");
        assert_eq!(logins[0].1["token"], "fixture-login-token");
        assert_eq!(logins[0].1["refresh_token"], true);
    }

    #[cfg(not(target_os = "ios"))]
    #[tokio::test]
    async fn browser_registration_sets_matrix_action_parameter() {
        let server = Server::new();
        let client = server.client().await;
        browser_login(&client, None, true, |link| async move {
            let url = Url::parse(&link).unwrap();
            assert!(url.query_pairs().any(|(key, value)| key == "action" && value == "register"));
            let redirect = url.query_pairs().find(|(key, _)| key == "redirectUrl").unwrap().1.into_owned();
            let mut callback = Url::parse(&redirect).unwrap();
            callback.query_pairs_mut().append_pair("loginToken", "fixture-login-token");
            matrix_sdk::reqwest::get(callback).await.unwrap().error_for_status().unwrap();
            Ok(())
        }).await.unwrap();
        assert!(client.matrix_auth().logged_in());
    }

    #[cfg(not(target_os = "ios"))]
    #[tokio::test]
    async fn cancellation_closes_callback_and_does_not_exchange_token() {
        let server = Server::new();
        let client = server.client().await;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            browser_login(&client, None, false, |link| async move {
                let url = Url::parse(&link).unwrap();
                let redirect = url
                    .query_pairs()
                    .find(|(k, _)| k == "redirectUrl")
                    .unwrap()
                    .1
                    .into_owned();
                sender.send(redirect).unwrap();
                Ok(())
            })
            .await
        });
        let redirect = receiver.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(matrix_sdk::reqwest::get(redirect).await.is_err());
        assert!(!server
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|(route, _)| route.starts_with("POST ") && route.contains("/login ")));
    }
}

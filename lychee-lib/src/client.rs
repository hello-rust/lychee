use std::{collections::HashSet, convert::TryFrom, time::Duration};

use check_if_email_exists::{check_email, CheckEmailInput, Reachable};
use derive_builder::Builder;
use http::{
    header::{HeaderMap, HeaderValue},
    StatusCode,
};
use hubcaps::{Credentials, Github};
use regex::RegexSet;
use reqwest::header;
use tokio::time::sleep;

use crate::{
    filter::{Excludes, Filter, Includes},
    quirks::Quirks,
    uri::Uri,
    ErrorKind, Request, Response, Result, Status,
};

const DEFAULT_MAX_REDIRECTS: usize = 5;
const DEFAULT_USER_AGENT: &str = concat!("lychee/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct Client {
    /// Underlying reqwest client instance that handles the HTTP requests.
    reqwest_client: reqwest::Client,
    /// Github client.
    github_client: Option<Github>,
    /// Filtered domain handling.
    filter: Filter,
    /// Default request HTTP method to use.
    method: reqwest::Method,
    /// The set of accepted HTTP status codes for valid URIs.
    accepted: Option<HashSet<StatusCode>>,
    /// Override behavior for certain known issues with URIs.
    quirks: Quirks,
}

/// A link checker using an API token for Github links
/// otherwise a normal HTTP client.
#[derive(Builder, Debug)]
#[builder(build_fn(skip))]
#[builder(setter(into))]
#[builder(name = "ClientBuilder")]
pub struct ClientBuilderInternal {
    /// Set an optional Github token.
    /// This allows for more requests before
    /// getting rate-limited.
    github_token: Option<String>,
    /// Check links matching this set of regular expressions
    includes: Option<RegexSet>,
    /// Exclude links matching this set of regular expressions
    excludes: Option<RegexSet>,
    /// Exclude all private network addresses
    exclude_all_private: bool,
    /// Exclude private IP addresses
    exclude_private_ips: bool,
    /// Exclude link-local IPs
    exclude_link_local_ips: bool,
    /// Exclude loopback IP addresses (e.g. 127.0.0.1)
    exclude_loopback_ips: bool,
    /// Don't check mail addresses
    exclude_mail: bool,
    /// Maximum number of redirects before returning error
    max_redirects: usize,
    /// User agent used for checking links
    user_agent: String,
    /// Ignore SSL errors
    allow_insecure: bool,
    /// Allowed URI scheme (e.g. https, http).
    /// This excludes all links from checking, which
    /// don't specify that scheme in the URL.
    scheme: Option<String>,
    /// Map of headers to send to each resource.
    /// This allows working around validation issues
    /// on some websites.
    custom_headers: HeaderMap,
    /// Request method (e.g. `GET` or `HEAD`)
    method: reqwest::Method,
    /// Set of accepted return codes / status codes
    accepted: Option<HashSet<StatusCode>>,
    /// Response timeout per request
    timeout: Option<Duration>,
}

impl ClientBuilder {
    fn build_excludes(&self) -> Excludes {
        // exclude_all_private option turns on all "private" excludes,
        // including private IPs, link-local IPs and loopback IPs
        let exclude_all_private = matches!(self.exclude_all_private, Some(true));
        let enable_exclude = |opt| exclude_all_private || matches!(opt, Some(true));

        Excludes {
            regex: self.excludes.clone().unwrap_or_default(),
            private_ips: enable_exclude(self.exclude_private_ips),
            link_local_ips: enable_exclude(self.exclude_link_local_ips),
            loopback_ips: enable_exclude(self.exclude_loopback_ips),
            mail: self.exclude_mail.unwrap_or_default(),
        }
    }

    fn build_includes(&self) -> Includes {
        let regex = self.includes.clone().flatten();
        Includes { regex }
    }

    /// The build method instantiates the client.
    pub fn build(&self) -> Result<Client> {
        // Faking the user agent is necessary for some websites, unfortunately.
        // Otherwise we get a 403 from the firewall (e.g. Sucuri/Cloudproxy on ldra.com).
        let user_agent = self
            .user_agent
            .clone()
            .unwrap_or_else(|| DEFAULT_USER_AGENT.to_owned());

        let mut headers = self.custom_headers.clone().unwrap_or_default();
        headers.insert(header::USER_AGENT, HeaderValue::from_str(&user_agent)?);
        headers.insert(
            header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );

        let allow_insecure = self.allow_insecure.unwrap_or(false);
        let max_redirects = self.max_redirects.unwrap_or(DEFAULT_MAX_REDIRECTS);

        let builder = reqwest::ClientBuilder::new()
            .gzip(true)
            .default_headers(headers)
            .danger_accept_invalid_certs(allow_insecure)
            .redirect(reqwest::redirect::Policy::limited(max_redirects));

        let timeout = self.timeout.flatten();

        let reqwest_client = (match timeout {
            Some(t) => builder.timeout(t),
            None => builder,
        })
        .build()?;

        let github_token = match self.github_token.clone().flatten() {
            Some(token) if !token.is_empty() => {
                Some(Github::new(user_agent, Credentials::Token(token))?)
            }
            _ => None,
        };

        let includes = self.build_includes();
        let excludes = self.build_excludes();
        let scheme = self.scheme.clone().flatten().map(|s| s.to_lowercase());

        let filter = Filter::new(Some(includes), Some(excludes), scheme);

        let quirks = Quirks::default();

        Ok(Client {
            reqwest_client,
            github_client: github_token,
            filter,
            quirks,
            method: self.method.clone().unwrap_or(reqwest::Method::GET),
            accepted: self.accepted.clone().unwrap_or_default(),
        })
    }
}

impl Client {
    pub async fn check<T, E>(&self, request: T) -> Result<Response>
    where
        Request: TryFrom<T, Error = E>,
        ErrorKind: From<E>,
    {
        let Request { uri, source } = Request::try_from(request)?;
        let status = if self.filter.is_excluded(&uri) {
            Status::Excluded
        } else if uri.scheme() == "mailto" {
            self.check_mail(&uri).await
        } else {
            self.check_website(&uri).await
        };

        Ok(Response::new(uri, status, source))
    }

    pub async fn check_website(&self, uri: &Uri) -> Status {
        let mut retries: i64 = 3;
        let mut wait: u64 = 1;

        let mut status = self.check_default(uri).await;
        while retries > 0 {
            if status.is_success() {
                return status;
            }
            retries -= 1;
            sleep(Duration::from_secs(wait)).await;
            wait *= 2;
            status = self.check_default(uri).await;
        }
        // Pull out the heavy weapons in case of a failed normal request.
        // This could be a Github URL and we run into the rate limiter.
        if let Some((owner, repo)) = uri.extract_github() {
            return self.check_github(owner, repo).await;
        }

        status
    }

    async fn check_github(&self, owner: &str, repo: &str) -> Status {
        match &self.github_client {
            Some(github) => github
                .repo(owner, repo)
                .get()
                .await
                .map_or_else(|e| e.into(), |_| Status::Ok(StatusCode::OK)),
            None => ErrorKind::MissingGitHubToken.into(),
        }
    }

    async fn check_default(&self, uri: &Uri) -> Status {
        let request = match self
            .reqwest_client
            .request(self.method.clone(), uri.as_str())
            .build()
        {
            Ok(r) => r,
            Err(e) => return e.into(),
        };

        let request = self.quirks.apply(request);

        match self.reqwest_client.execute(request).await {
            Ok(ref response) => Status::new(response, self.accepted.clone()),
            Err(e) => e.into(),
        }
    }

    pub async fn check_mail(&self, uri: &Uri) -> Status {
        let input = CheckEmailInput::new(vec![uri.as_str().to_owned()]);
        let result = &(check_email(&input).await)[0];

        if let Reachable::Invalid = result.is_reachable {
            ErrorKind::UnreachableEmailAddress(uri.clone()).into()
        } else {
            Status::Ok(StatusCode::OK)
        }
    }
}

/// A convenience function to check a single URI
/// This is the most simple link check and avoids having to create a client manually.
/// For more complex scenarios, look into using the [`ClientBuilder`] instead.
pub async fn check<T, E>(request: T) -> Result<Response>
where
    Request: TryFrom<T, Error = E>,
    ErrorKind: From<E>,
{
    let client = ClientBuilder::default().build()?;
    Ok(client.check(request).await?)
}

#[cfg(test)]
mod test {
    use std::time::{Duration, Instant};

    use http::{header::HeaderMap, StatusCode};
    use reqwest::header;

    use super::ClientBuilder;
    use crate::{mock_server, test_utils::get_mock_client_response};

    #[tokio::test]
    async fn test_nonexistent() {
        let mock_server = mock_server!(StatusCode::NOT_FOUND);
        let res = get_mock_client_response(mock_server.uri()).await;

        assert!(res.status().is_failure());
    }

    #[tokio::test]
    async fn test_nonexistent_with_path() {
        let res = get_mock_client_response("http://127.0.0.1/invalid").await;

        assert!(res.status().is_failure());
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let mock_server = mock_server!(StatusCode::NOT_FOUND);

        let start = Instant::now();
        let res = get_mock_client_response(mock_server.uri()).await;
        let end = start.elapsed();

        assert!(res.status().is_failure());

        // on slow connections, this might take a bit longer than nominal backed-off timeout (7 secs)
        assert!(end.as_secs() >= 7);
        assert!(end.as_secs() <= 8);
    }

    #[tokio::test]
    async fn test_github() {
        let res = get_mock_client_response("https://github.com/lycheeverse/lychee").await;

        assert!(res.status().is_success());
    }

    #[tokio::test]
    async fn test_github_nonexistent() {
        let res = get_mock_client_response("https://github.com/lycheeverse/not-lychee").await;

        assert!(res.status().is_failure());
    }

    #[tokio::test]
    async fn test_youtube() {
        // This is applying a quirk. See the quirks module.
        let res = get_mock_client_response("https://www.youtube.com/watch?v=NlKuICiT470&list=PLbWDhxwM_45mPVToqaIZNbZeIzFchsKKQ&index=7").await;
        assert!(res.status().is_success());

        let res = get_mock_client_response("https://www.youtube.com/watch?v=invalidNlKuICiT470&list=PLbWDhxwM_45mPVToqaIZNbZeIzFchsKKQ&index=7").await;
        assert!(res.status().is_failure());
    }

    #[tokio::test]
    async fn test_non_github() {
        let mock_server = mock_server!(StatusCode::OK);
        let res = get_mock_client_response(mock_server.uri()).await;

        assert!(res.status().is_success());
    }

    #[tokio::test]
    async fn test_invalid_ssl() {
        let res = get_mock_client_response("https://expired.badssl.com/").await;

        assert!(res.status().is_failure());

        // Same, but ignore certificate error
        let res = ClientBuilder::default()
            .allow_insecure(true)
            .build()
            .unwrap()
            .check("https://expired.badssl.com/")
            .await
            .unwrap();
        assert!(res.status().is_success());
    }

    #[tokio::test]
    async fn test_custom_headers() {
        let res = get_mock_client_response("https://crates.io/crates/lychee/").await;

        assert!(res.status().is_failure());

        // Try again, but with a custom header.
        // For example, crates.io requires a custom accept header.
        // See https://github.com/rust-lang/crates.io/issues/788
        let mut custom = HeaderMap::new();
        custom.insert(header::ACCEPT, "text/html".parse().unwrap());
        let res = ClientBuilder::default()
            .custom_headers(custom)
            .build()
            .unwrap()
            .check("https://crates.io/crates/lychee")
            .await
            .unwrap();
        assert!(res.status().is_success());
    }

    #[tokio::test]
    async fn test_timeout() {
        // Note: this checks response timeout, not connect timeout.
        // To check connect timeout, we'd have to do something more involved,
        // see: https://github.com/LukeMathWalker/wiremock-rs/issues/19
        let mock_delay = Duration::from_millis(20);
        let checker_timeout = Duration::from_millis(10);
        assert!(mock_delay > checker_timeout);

        let mock_server = mock_server!(StatusCode::OK, set_delay(mock_delay));

        let client = ClientBuilder::default()
            .timeout(checker_timeout)
            .build()
            .unwrap();

        let res = client.check(mock_server.uri()).await.unwrap();
        assert!(res.status().is_timeout());
    }
}

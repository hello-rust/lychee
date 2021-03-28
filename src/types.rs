use crate::{collector::Input, uri::Uri};
use anyhow::anyhow;
use serde::{Serialize, Serializer};
use std::{collections::HashSet, convert::TryFrom, fmt::Display};

pub type Cache = HashSet<String>;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Request {
    pub uri: Uri,
    pub source: Input,
    pub recursion_level: usize,
}

impl Request {
    pub fn new(uri: Uri, source: Input, recursion_level: usize) -> Self {
        Request {
            uri,
            source,
            recursion_level,
        }
    }
}

impl Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.uri, self.source)
    }
}

impl TryFrom<String> for Request {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        let uri = Uri::try_from(s.as_str())?;
        Ok(Request::new(uri, Input::String(s), 0))
    }
}

impl TryFrom<&str> for Request {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        let uri = Uri::try_from(s)?;
        Ok(Request::new(uri, Input::String(s.to_owned()), 0))
    }
}

/// Specifies how requests to websites will be made
pub(crate) enum RequestMethod {
    Get,
    Head,
}

impl TryFrom<String> for RequestMethod {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_ref() {
            "get" => Ok(RequestMethod::Get),
            "head" => Ok(RequestMethod::Head),
            _ => Err(anyhow!("Only `get` and `head` allowed, got {}", value)),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Serialize, Clone)]
pub struct Response {
    #[serde(flatten)]
    pub uri: Uri,
    pub status: Status,
    #[serde(skip)]
    pub source: Input,
    #[serde(skip)]
    pub recursion_level: usize,
}

impl Response {
    pub fn new(uri: Uri, status: Status, source: Input, recursion_level: usize) -> Self {
        Response {
            uri,
            status,
            source,
            recursion_level,
        }
    }
}

impl Display for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let metadata = match &self.status {
            Status::Ok(code) | Status::Redirected(code) => {
                format!(" [{}]", code)
            }
            Status::Timeout(code) if code.is_some() => format!(" [{}]", code.unwrap()),
            Status::Error(e, code) => {
                if let Some(code) = code {
                    format!(" ({})", code)
                } else {
                    format!(" ({})", e)
                }
            }
            _ => "".to_string(),
        };
        write!(f, "{} {}{}", self.status.icon(), self.uri, metadata)
    }
}

/// Response status of the request
#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub enum Status {
    /// Request was successful
    Ok(http::StatusCode),
    /// Request failed with HTTP error code
    Error(String, Option<http::StatusCode>),
    /// Request timed out
    Timeout(Option<http::StatusCode>),
    /// Got redirected to different resource
    Redirected(http::StatusCode),
    /// Resource was excluded from checking
    Excluded,
}

impl Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let out = match self {
            Status::Ok(c) => format!("OK ({})", c),
            Status::Redirected(c) => format!("Redirect ({})", c),
            Status::Excluded => "Excluded".to_string(),
            Status::Error(err, code) => {
                if let Some(code) = code {
                    format!("Failed: {} ({})", err, code)
                } else {
                    format!("Failed: {}", err)
                }
            }
            Status::Timeout(Some(c)) => format!("Timeout ({})", c),
            Status::Timeout(None) => "Timeout".to_string(),
        };
        write!(f, "{}", out)
    }
}

impl Serialize for Status {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl Status {
    pub fn new(statuscode: http::StatusCode, accepted: Option<HashSet<http::StatusCode>>) -> Self {
        if let Some(true) = accepted.map(|a| a.contains(&statuscode)) {
            Status::Ok(statuscode)
        } else if statuscode.is_success() {
            Status::Ok(statuscode)
        } else if statuscode.is_redirection() {
            Status::Redirected(statuscode)
        } else {
            Status::Error("".into(), Some(statuscode))
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Status::Ok(_))
    }

    pub fn is_failure(&self) -> bool {
        matches!(self, Status::Error(_, _))
    }

    pub fn is_excluded(&self) -> bool {
        matches!(self, Status::Excluded)
    }

    pub fn icon(&self) -> &str {
        match self {
            Status::Ok(_) => "✔",
            Status::Redirected(_) => "⇄️",
            Status::Excluded => "?",
            Status::Error(_, _) => "✗",
            Status::Timeout(_) => "⧖",
        }
    }
}

impl From<reqwest::Error> for Status {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            Status::Timeout(e.status())
        } else {
            Status::Error(e.to_string(), e.status())
        }
    }
}

#[cfg(test)]
mod test {
    use crate::test_utils::website;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_uri_host_ip_v4() {
        let uri = website("http://127.0.0.1");
        let ip = uri.host_ip().expect("Expected a valid IPv4");
        assert_eq!(ip, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn test_uri_host_ip_v6() {
        let uri = website("https://[2020::0010]");
        let ip = uri.host_ip().expect("Expected a valid IPv6");
        assert_eq!(
            ip,
            IpAddr::V6(Ipv6Addr::new(0x2020, 0, 0, 0, 0, 0, 0, 0x10))
        );
    }

    #[test]
    fn test_uri_host_ip_no_ip() {
        let uri = website("https://some.cryptic/url");
        let ip = uri.host_ip();
        assert!(ip.is_none());
    }
}

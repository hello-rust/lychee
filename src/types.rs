use crate::options::Config;
use anyhow::anyhow;
use std::{collections::HashSet, convert::TryFrom};

use regex::RegexSet;

use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Uri {
    Website(Url),
    Mail(String),
}

/// Specifies how requests to websites will be made
pub(crate) enum RequestMethod {
    GET,
    HEAD,
}

impl TryFrom<String> for RequestMethod {
    type Error = anyhow::Error;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.to_lowercase().as_ref() {
            "get" => Ok(RequestMethod::GET),
            "head" => Ok(RequestMethod::HEAD),
            _ => Err(anyhow!("Only `get` and `head` allowed, got {}", value)),
        }
    }
}

#[derive(Debug)]
pub struct Response {
    pub uri: Uri,
    pub status: Status,
}

impl Response {
    pub fn new(uri: Uri, status: Status) -> Self {
        Response { uri, status }
    }
}

/// Response status of the request
#[derive(Debug)]
pub enum Status {
    /// Request was successful
    Ok(http::StatusCode),
    /// Request failed with HTTP error code
    Failed(http::StatusCode),
    /// Request timed out
    Timeout,
    /// Got redirected to different resource
    Redirected,
    /// Resource was excluded from checking
    Excluded,
    /// Low-level error while loading resource
    Error(String),
}

impl Status {
    pub fn new(statuscode: http::StatusCode, accepted: Option<HashSet<http::StatusCode>>) -> Self {
        if let Some(accepted) = accepted {
            if accepted.contains(&statuscode) {
                return Status::Ok(statuscode);
            }
        } else if statuscode.is_success() {
            return Status::Ok(statuscode);
        };
        if statuscode.is_redirection() {
            Status::Redirected
        } else {
            Status::Failed(statuscode)
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Status::Ok(_))
    }
}

impl From<reqwest::Error> for Status {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            Status::Timeout
        } else {
            Status::Error(e.to_string())
        }
    }
}

/// Exclude configuration for the link checker.
/// You can ignore links based on
#[derive(Clone, Debug)]
pub struct Excludes {
    pub regex: Option<RegexSet>,
    /// Example: 192.168.0.1
    pub private_ips: bool,
    /// Example: 169.254.0.0
    pub link_local_ips: bool,
    /// For IPv4: 127.0. 0.1/8
    /// For IPv6: ::1/128
    pub loopback_ips: bool,
}

impl Excludes {
    pub fn from_options(config: &Config) -> Self {
        // exclude_all_private option turns on all "private" excludes,
        // including private IPs, link-local IPs and loopback IPs
        let enable_exclude = |opt| opt || config.exclude_all_private;

        Self {
            regex: RegexSet::new(&config.exclude).ok(),
            private_ips: enable_exclude(config.exclude_private),
            link_local_ips: enable_exclude(config.exclude_link_local),
            loopback_ips: enable_exclude(config.exclude_loopback),
        }
    }
}

impl Default for Excludes {
    fn default() -> Self {
        Self {
            regex: None,
            private_ips: false,
            link_local_ips: false,
            loopback_ips: false,
        }
    }
}

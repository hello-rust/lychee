#[deny(missing_docs)]

/**
* `lychee` is a library for checking links.
* The main struct of this crate is `ClientBuilder` which can be used to
* configure and run your own link checker.
*
* "Hello world" example:
* ```
*
* use lychee::{ClientBuilder, Status};
* use lychee::Uri::Website;
* use url::Url;
* use std::error::Error;
*
* #[tokio::main]
* async fn main() -> Result<(), Box<dyn Error>> {
*   let client = ClientBuilder::default().build()?;
*   let url = Url::parse("https://github.com/lycheeverse/lychee")?;
*   let response = client.check(Website(url)).await;
*   assert!(matches!(response.status, Status::Ok(_)));
*   Ok(())
* }
* ```
*/
mod client;
mod client_pool;
mod excludes;
mod test_utils;
mod types;
mod uri;

pub use client::ClientBuilder;
pub use client_pool::ClientPool;
pub use excludes::Excludes;
pub use types::*;
pub use uri::Uri;

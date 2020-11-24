use std::{
    collections::HashSet,
    fmt::{self, Display},
};

use crate::types::Response;
use crate::types::Uri;

pub struct ResponseStats {
    total: usize,
    successful: usize,
    failed: HashSet<Uri>,
    timeout: HashSet<Uri>,
    redirected: HashSet<Uri>,
    excluded: HashSet<Uri>,
    error: HashSet<Uri>,
}

impl ResponseStats {
    pub fn new() -> Self {
        ResponseStats {
            total: 0,
            successful: 0,
            failed: HashSet::new(),
            timeout: HashSet::new(),
            redirected: HashSet::new(),
            excluded: HashSet::new(),
            error: HashSet::new(),
        }
    }

    pub fn add(&mut self, response: Response) {
        self.total += 1;
        let uri = response.uri;
        match response.status {
            crate::types::Status::Ok(_) => self.successful += 1,
            crate::types::Status::Failed(_) => {
                self.failed.insert(uri);
            }
            crate::types::Status::Timeout => {
                self.timeout.insert(uri);
            }
            crate::types::Status::Redirected => {
                self.redirected.insert(uri);
            }
            crate::types::Status::Excluded => {
                self.excluded.insert(uri);
            }
            crate::types::Status::Error(_) => {
                self.error.insert(uri);
            }
        };
    }

    pub fn is_success(&self) -> bool {
        self.total == self.successful + self.excluded.len()
    }
}

impl Display for ResponseStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "📝 Summary")?;
        writeln!(f, "-------------------")?;
        writeln!(f, "🔍 Total: {}", self.total)?;
        writeln!(f, "✅ Successful: {}", self.successful)?;
        writeln!(f, "⏳ Timeout: {}", self.timeout.len())?;
        writeln!(f, "🔀 Redirected: {}", self.redirected.len())?;
        writeln!(f, "👻 Excluded: {}", self.excluded.len())?;
        writeln!(f, "🚫 Errors: {}", self.error.len() + self.failed.len())?;
        Ok(())
    }
}

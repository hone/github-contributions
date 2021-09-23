use octocrab::models::User;
use serde::Deserialize;
use std::{
    cmp::{PartialEq, PartialOrd},
    fmt,
};

#[derive(Clone, Debug, Deserialize, PartialEq, PartialOrd, Eq, Hash)]
pub struct Repo {
    pub org: String,
    pub name: String,
}

impl Repo {
    pub fn new(org: impl Into<String>, name: impl Into<String>) -> Self {
        Repo {
            org: org.into(),
            name: name.into(),
        }
    }
}

impl fmt::Display for Repo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.org, self.name)
    }
}

#[derive(Deserialize)]
pub struct EnrichedUser {
    #[serde(flatten)]
    pub inner: User,
    pub company: Option<String>,
    pub email: Option<String>,
}

impl fmt::Debug for EnrichedUser {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnrichedUser")
            .field("login", &self.inner.login)
            .field("id", &self.inner.id)
            .field("company", &self.company)
            .field("email", &self.email)
            .finish()
    }
}

pub mod commit {
    use chrono::{offset::Utc, DateTime};
    use octocrab::models::repos::Commit;
    use serde::Deserialize;

    #[derive(Clone, Debug, Deserialize)]
    pub struct EnrichedCommit {
        #[serde(flatten)]
        pub inner: Commit,
        pub commit: CommitObject,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct CommitObject {
        pub author: Author,
    }

    #[derive(Clone, Debug, Deserialize)]
    pub struct Author {
        pub name: String,
        pub email: String,
        pub date: DateTime<Utc>,
    }
}

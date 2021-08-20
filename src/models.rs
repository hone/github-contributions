use octocrab::models::User;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
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

#[derive(Debug, Deserialize)]
pub struct EnrichedUser {
    #[serde(flatten)]
    pub inner: User,
    pub company: Option<String>,
    pub email: Option<String>,
}

pub mod commit {
    use chrono::{offset::Utc, DateTime};
    use octocrab::models::repos::Commit;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    pub struct EnrichedCommit {
        #[serde(flatten)]
        pub inner: Commit,
        pub commit: CommitObject,
    }

    #[derive(Debug, Deserialize)]
    pub struct CommitObject {
        pub author: Author,
    }

    #[derive(Debug, Deserialize)]
    pub struct Author {
        pub name: String,
        pub email: String,
        pub date: DateTime<Utc>,
    }
}

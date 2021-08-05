use chrono::{offset::Utc, DateTime};
use octocrab::models::{issues::Issue, pulls::Review, User};

/// GitHub Contribution as defined in the [GitHub documentation](https://docs.github.com/en/github/setting-up-and-managing-your-github-profile/managing-contribution-graphs-on-your-profile/viewing-contributions-on-your-profile#what-counts-as-a-contribution).
pub enum Contribution {
    Issue(Issue),
    Review(Review),
}

impl Contribution {
    pub fn created_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::Issue(issue) => Some(issue.created_at),
            Self::Review(review) => review.submitted_at,
        }
    }

    pub fn user(&self) -> &User {
        match self {
            Self::Issue(issue) => &issue.user,
            Self::Review(review) => &review.user,
        }
    }
}

impl From<Issue> for Contribution {
    fn from(issue: Issue) -> Contribution {
        Contribution::Issue(issue)
    }
}

impl From<Review> for Contribution {
    fn from(review: Review) -> Contribution {
        Contribution::Review(review)
    }
}

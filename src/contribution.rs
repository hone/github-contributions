use chrono::{offset::Utc, DateTime};
use octocrab::models::{issues::Issue, pulls::Review, repos::Commit, User};

/// GitHub Contribution as defined in the [GitHub documentation](https://docs.github.com/en/github/setting-up-and-managing-your-github-profile/managing-contribution-graphs-on-your-profile/viewing-contributions-on-your-profile#what-counts-as-a-contribution).
pub enum Contribution {
    Commit(Commit),
    Issue(Issue),
    Review(Review),
}

impl Contribution {
    pub fn created_at(&self) -> Option<DateTime<Utc>> {
        match self {
            // Pull this from author
            Self::Commit(_) => None,
            Self::Issue(issue) => Some(issue.created_at),
            Self::Review(review) => review.submitted_at,
        }
    }

    pub fn user(&self) -> Option<&User> {
        match self {
            Self::Commit(commit) => commit.author.as_ref(),
            Self::Issue(issue) => Some(&issue.user),
            Self::Review(review) => Some(&review.user),
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

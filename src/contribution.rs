use crate::github_contribution_collector::CommitWithDate;
use chrono::{offset::Utc, DateTime};
use octocrab::models::{issues::Issue, pulls::Review, User};

/// GitHub Contribution as defined in the [GitHub documentation](https://docs.github.com/en/github/setting-up-and-managing-your-github-profile/managing-contribution-graphs-on-your-profile/viewing-contributions-on-your-profile#what-counts-as-a-contribution).
#[derive(Debug)]
pub enum Contribution {
    Commit(CommitWithDate),
    Issue(Issue),
    Review(Review),
}

impl Contribution {
    pub fn created_at(&self) -> Option<DateTime<Utc>> {
        match self {
            // Pull this from author
            Self::Commit(commit) => Some(commit.commit.author.date),
            Self::Issue(issue) => Some(issue.created_at),
            Self::Review(review) => review.submitted_at,
        }
    }

    pub fn user(&self) -> Option<&User> {
        match self {
            Self::Commit(commit) => commit.inner.author.as_ref(),
            Self::Issue(issue) => Some(&issue.user),
            Self::Review(review) => Some(&review.user),
        }
    }
}

impl From<CommitWithDate> for Contribution {
    fn from(commit: CommitWithDate) -> Contribution {
        Contribution::Commit(commit)
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

use crate::models::{commit::EnrichedCommit, Repo};
use chrono::{offset::Utc, DateTime};
use octocrab::models::{issues::Issue, pulls::Review, User};

#[derive(Clone, Debug)]
pub struct Contribution {
    pub repo: Repo,
    pub contribution: GithubContribution,
}

impl Contribution {
    pub fn new(
        org: impl Into<String>,
        repo: impl Into<String>,
        contribution: GithubContribution,
    ) -> Self {
        Contribution {
            repo: Repo::new(org.into(), repo.into()),
            contribution,
        }
    }
}

/// GitHub Contribution as defined in the [GitHub documentation](https://docs.github.com/en/github/setting-up-and-managing-your-github-profile/managing-contribution-graphs-on-your-profile/viewing-contributions-on-your-profile#what-counts-as-a-contribution).
#[derive(Clone, Debug)]
pub enum GithubContribution {
    Commit(EnrichedCommit),
    Issue(Issue),
    Review(Review),
}

impl Contribution {
    pub fn created_at(&self) -> Option<DateTime<Utc>> {
        match &self.contribution {
            // Pull this from author
            GithubContribution::Commit(commit) => Some(commit.commit.author.date),
            GithubContribution::Issue(issue) => Some(issue.created_at),
            GithubContribution::Review(review) => review.submitted_at,
        }
    }

    pub fn user(&self) -> Option<&User> {
        match &self.contribution {
            GithubContribution::Commit(commit) => commit.inner.author.as_ref(),
            GithubContribution::Issue(issue) => Some(&issue.user),
            GithubContribution::Review(review) => Some(&review.user),
        }
    }
}

impl From<EnrichedCommit> for GithubContribution {
    fn from(commit: EnrichedCommit) -> GithubContribution {
        GithubContribution::Commit(commit)
    }
}

impl From<Issue> for GithubContribution {
    fn from(issue: Issue) -> GithubContribution {
        GithubContribution::Issue(issue)
    }
}

impl From<Review> for GithubContribution {
    fn from(review: Review) -> GithubContribution {
        GithubContribution::Review(review)
    }
}

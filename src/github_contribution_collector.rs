use crate::Contribution;
use chrono::{offset::Utc, DateTime};
use octocrab::{
    models::{issues::Issue, pulls::Review, repos::Commit, User},
    params, Page,
};
use serde::{de::DeserializeOwned, Deserialize};
use std::collections::HashMap;
use tokio_stream::StreamExt;

#[derive(Debug, Deserialize)]
pub struct UserWithCompanyInfo {
    #[serde(flatten)]
    pub inner: User,
    pub company: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommitWithDate {
    #[serde(flatten)]
    pub inner: Commit,
    pub commit: CommitCommit,
}

#[derive(Debug, Deserialize)]
pub struct CommitCommit {
    pub author: CommitAuthor,
}

#[derive(Debug, Deserialize)]
pub struct CommitAuthor {
    pub name: String,
    pub email: String,
    pub date: DateTime<Utc>,
}

#[derive(Debug)]
pub struct Output {
    pub user: Option<UserWithCompanyInfo>,
    pub membership: bool,
    pub contributions: Vec<Contribution>,
}

pub struct GithubContributionCollector {
    client: octocrab::Octocrab,
}

impl GithubContributionCollector {
    pub fn new(token: Option<impl Into<String>>) -> Result<Self, octocrab::Error> {
        let mut client = octocrab::OctocrabBuilder::new();
        if let Some(token) = token {
            client = client.personal_token(token.into());
        }

        Ok(Self {
            client: client.build()?,
        })
    }

    /// Given an Iterator of `Contribution`s, generate a `Vec<Output>`.
    pub async fn process_contributions(
        &self,
        contributions: impl Iterator<Item = Contribution>,
        company_orgs: Vec<impl AsRef<str>>,
    ) -> Result<Vec<Output>, octocrab::Error> {
        let mut user_contributions: HashMap<Option<User>, Vec<Contribution>> = HashMap::new();
        for contribution in contributions {
            // the hidden `query` field in `User` can be different so will create different keys in the HashMap
            let entry = if let Some(user) = user_contributions
                .keys()
                .find(|user| user.as_ref().map(|u| u.id) == contribution.user().map(|u| u.id))
            {
                // mutable_borrow_reservation_conflict: https://github.com/rust-lang/rust/issues/59159
                let key = user.clone();
                user_contributions.entry(key)
            } else {
                user_contributions.entry(contribution.user().map(|u| u.clone()))
            };
            let value = entry.or_insert(Vec::new());
            (*value).push(contribution);
        }

        let output_stream = tokio_stream::iter(user_contributions)
            .map(|(maybe_user, contributions)| {
                // create copies so they can be moved inside the async block
                let cloned_client = self.client.clone();
                let orgs = company_orgs
                    .iter()
                    .map(|org| self.client.orgs(org.as_ref()));

                async move {
                    let mut membership = false;
                    let mut maybe_company_user = None;

                    if let Some(user) = maybe_user {
                        let company_user: UserWithCompanyInfo = cloned_client
                            .get(format!("/users/{}", user.login), None::<&()>)
                            .await?;
                        for org in orgs {
                            membership = membership
                                || org.check_membership(&company_user.inner.login).await?;
                        }
                        maybe_company_user = Some(company_user);
                    }

                    Result::<_, octocrab::Error>::Ok(Output {
                        user: maybe_company_user,
                        membership,
                        contributions,
                    })
                }
            })
            .collect::<Vec<_>>()
            .await;

        Ok(futures::future::try_join_all(output_stream).await?)
    }

    /// Collect all contributions from commits on the default branch associated with this repo.
    pub async fn commits(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<CommitWithDate>, octocrab::Error> {
        let page: Page<CommitWithDate> = self
            .client
            .get(
                format!("/repos/{}/{}/commits", repo_org.as_ref(), repo.as_ref()),
                None::<&()>,
            )
            .await?;

        Ok(self.process_pages(page).await?)
    }

    /// Collect all contributions from issues associated with this repo.
    pub async fn issues(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Issue>, octocrab::Error> {
        let page = self
            .client
            .issues(repo_org.as_ref(), repo.as_ref())
            .list()
            .sort(params::issues::Sort::Created)
            .direction(params::Direction::Descending)
            .send()
            .await?;

        Ok(self.process_pages(page).await?)
    }

    /// Collect all contributions reviews associated with this repo.
    pub async fn reviews(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Review>, octocrab::Error> {
        let pull_handler = self.client.pulls(repo_org.as_ref(), repo.as_ref());
        let page = pull_handler
            .list()
            .sort(params::pulls::Sort::Created)
            .direction(params::Direction::Descending)
            .send()
            .await?;

        let pull_requests = self.process_pages(page).await?;

        let reviews_stream = tokio_stream::iter(pull_requests)
            .map(|pull_request| {
                let pull_handler = self.client.pulls(repo_org.as_ref(), repo.as_ref());

                async move {
                    let page = pull_handler.list_reviews(pull_request.number).await?;

                    Result::<_, octocrab::Error>::Ok(self.process_pages(page).await?)
                }
            })
            .collect::<Vec<_>>()
            .await;

        Ok(futures::future::try_join_all(reviews_stream)
            .await?
            .into_iter()
            .flatten()
            .collect())
    }

    /// Get all the items from the current page until the end
    async fn process_pages<T: DeserializeOwned>(
        &self,
        mut page: Page<T>,
    ) -> Result<Vec<T>, octocrab::Error> {
        let mut items = Vec::new();

        loop {
            for item in page.take_items() {
                items.push(item);
            }

            let next_page_option = self.client.get_page::<T>(&page.next).await?;
            if let Some(next_page) = next_page_option {
                page = next_page;
            } else {
                break;
            }
        }

        Ok(items)
    }
}

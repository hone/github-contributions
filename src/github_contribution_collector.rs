use crate::{
    models::{commit::EnrichedCommit, EnrichedUser},
    Contribution,
};
use async_stream::try_stream;
use futures::Stream;
use octocrab::{
    models::{
        issues::Issue,
        pulls::{PullRequest, Review},
        User,
    },
    orgs::OrgHandler,
    params, Page,
};
use regex::Regex;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_stream::StreamExt;

#[derive(Debug)]
pub struct Output {
    pub user: Option<EnrichedUser>,
    pub membership: bool,
    pub exclude: bool,
    pub contributions: Vec<Contribution>,
}

/// Client for getting different types of GitHub contributions
pub struct GithubContributionCollector {
    client: Arc<octocrab::Octocrab>,
}

#[derive(Debug, Clone)]
/// Regexes for doing exclude checks
struct ExcludeRegex {
    pub company: Regex,
    pub email: Regex,
}

impl GithubContributionCollector {
    pub fn new(token: Option<impl Into<String>>) -> Result<Self, octocrab::Error> {
        let mut client = octocrab::OctocrabBuilder::new();
        if let Some(token) = token {
            client = client.personal_token(token.into());
        }

        Ok(Self {
            client: Arc::new(client.build()?),
        })
    }

    /// Given an Iterator of `Contribution`s, generate a `Vec<Output>`.
    /// ## Notes
    /// `company_orgs` requires access from the GitHub Token provided.
    pub async fn process_contributions(
        &self,
        contributions: impl Iterator<Item = Contribution>,
        company_orgs: Vec<impl AsRef<str>>,
        exclude_orgs: Vec<impl AsRef<str> + Clone>,
    ) -> Result<Vec<Output>, octocrab::Error> {
        // build the regexes once before the stream and clone for perf reasons
        // https://github.com/rust-lang/regex/blob/master/PERFORMANCE.md#using-a-regex-from-multiple-threads
        let exclude_orgs_re = exclude_orgs.iter().map(|org| ExcludeRegex {
            company: Regex::new(format!(r#"(?i){}"#, regex::escape(org.as_ref())).as_str())
                .unwrap(),
            email: Regex::new(
                format!(r#"@(\w\.)*(?i){}(?-i)\."#, regex::escape(org.as_ref())).as_str(),
            )
            .unwrap(),
        });

        let collection = output_stream(
            self.client.clone(),
            contributions,
            company_orgs,
            exclude_orgs_re.clone(),
        )
        .await
        .collect::<Result<Vec<Output>, octocrab::Error>>()
        .await?;

        Ok(collection)
    }

    /// Return all contributions for a repo.
    pub async fn contributions(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Contribution>, octocrab::Error> {
        let (issues, reviews, commits) = tokio::join!(
            self.issues(repo_org.as_ref(), repo.as_ref()),
            self.reviews(repo_org.as_ref(), repo.as_ref()),
            self.commits(repo_org.as_ref(), repo.as_ref()),
        );

        let contributions = issues?
            .into_iter()
            .map(|issue| issue.into())
            .chain(reviews?.into_iter().map(|review| review.into()))
            .chain(commits?.into_iter().map(|commit| commit.into()))
            .collect();

        Ok(contributions)
    }

    /// Collect all contributions from commits on the default branch associated with this repo.
    pub async fn commits(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<EnrichedCommit>, octocrab::Error> {
        let page: Page<EnrichedCommit> = self
            .client
            .get(
                format!("/repos/{}/{}/commits", repo_org.as_ref(), repo.as_ref()),
                None::<&()>,
            )
            .await?;

        Ok(self.process_pages(page).await?)
    }

    pub async fn commits_as_contributions(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Contribution>, octocrab::Error> {
        Ok(self
            .commits(repo_org, repo)
            .await?
            .into_iter()
            .map(|commit| commit.into())
            .collect())
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

    pub async fn issues_as_contributions(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Contribution>, octocrab::Error> {
        Ok(self
            .issues(repo_org, repo)
            .await?
            .into_iter()
            .map(|issue| issue.into())
            .collect())
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

        let reviews = review_stream(
            self.client.clone(),
            pull_requests.into_iter(),
            repo_org,
            repo,
        )
        .await
        .collect::<Result<Vec<Vec<Review>>, octocrab::Error>>()
        .await?
        .into_iter()
        .flatten()
        .collect();

        Ok(reviews)
    }

    pub async fn reviews_as_contributions(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Contribution>, octocrab::Error> {
        Ok(self
            .reviews(repo_org, repo)
            .await?
            .into_iter()
            .map(|review| review.into())
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

/// Given an `Iterator` of Contributions, return a HashMap where the key is the User and the value is
/// a `Vec` of those contributions.
fn contributions_by_user(
    contributions: impl Iterator<Item = Contribution>,
) -> HashMap<Option<User>, Vec<Contribution>> {
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

    user_contributions
}

/// Use GitHub API to check membership. This requires the client TOKEN to have access to the org.
async fn check_membership(
    login: impl AsRef<str>,
    orgs: impl Iterator<Item = OrgHandler<'_>>,
) -> Result<bool, octocrab::Error> {
    let mut membership = false;

    for org in orgs {
        membership = membership || org.check_membership(login.as_ref()).await?;
    }

    Ok(membership)
}

/// Enrich user with more data from the GitHub API
async fn enrich_user(
    client: &octocrab::Octocrab,
    user: User,
) -> Result<EnrichedUser, octocrab::Error> {
    let enriched_user = match client
        .get(format!("/users/{}", &user.login), None::<&()>)
        .await
    {
        Ok(u) => Ok(u),
        Err(err) => match err {
            // for cases when the user can not be found (like `invalid-email-address` from the GitHub API).
            octocrab::Error::GitHub { source, backtrace } => {
                if source.documentation_url
                    == "https://docs.github.com/rest/reference/users#get-a-user"
                {
                    eprintln!("Could not fetch user: {}", &user.login);
                    Ok(EnrichedUser {
                        inner: user,
                        company: None,
                        email: None,
                    })
                } else {
                    Err(octocrab::Error::GitHub { source, backtrace })
                }
            }
            _ => Err(err),
        },
    }?;

    Ok(enriched_user)
}

/// Get all the items from the current page until the end
async fn process_pages<T: DeserializeOwned>(
    client: &octocrab::Octocrab,
    mut page: Page<T>,
) -> Result<Vec<T>, octocrab::Error> {
    let mut items = Vec::new();

    loop {
        for item in page.take_items() {
            items.push(item);
        }

        let next_page_option = client.get_page::<T>(&page.next).await?;
        if let Some(next_page) = next_page_option {
            page = next_page;
        } else {
            break;
        }
    }

    Ok(items)
}

async fn review_stream(
    client: Arc<octocrab::Octocrab>,
    pull_requests: impl Iterator<Item = PullRequest>,
    repo_org: impl AsRef<str>,
    repo: impl AsRef<str>,
) -> impl Stream<Item = Result<Vec<Review>, octocrab::Error>> {
    try_stream! {
        for pull_request in pull_requests {
            let pull_handler = &client.pulls(repo_org.as_ref(), repo.as_ref());
            let page = pull_handler.list_reviews(pull_request.number).await?;
            let items = process_pages(&client, page).await?;

            yield items;
        }
    }
}

/// Build an output stream
async fn output_stream(
    client: Arc<octocrab::Octocrab>,
    contributions: impl Iterator<Item = Contribution>,
    company_orgs: Vec<impl AsRef<str>>,
    exclude_orgs_re: impl Iterator<Item = ExcludeRegex> + Clone,
) -> impl Stream<Item = Result<Output, octocrab::Error>> {
    try_stream! {
        let orgs = company_orgs
            .iter()
            .map(|org| client.orgs(org.as_ref()));

        for (maybe_user, contributions) in contributions_by_user(contributions) {
            let mut membership = false;
            let mut exclude = false;
            let mut maybe_company_user = None;

            if let Some(user) = maybe_user {
                let enriched_user = enrich_user(&client, user).await?;
                membership = check_membership(&enriched_user.inner.login, orgs.clone()).await?;
                exclude = exclude_orgs_re.clone().any(|exclude_re| {
                    enriched_user
                        .company
                        .as_ref()
                        .map(|company| exclude_re.company.is_match(&company))
                        .unwrap_or(false)
                        || enriched_user
                        .email
                        .as_ref()
                        .map(|email| exclude_re.email.is_match(email))
                        .unwrap_or(false)
                });
                maybe_company_user = Some(enriched_user);
            }

            yield Output {
                user: maybe_company_user,
                membership,
                exclude,
                contributions,
            };
        }
    }
}

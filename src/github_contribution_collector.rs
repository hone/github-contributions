use crate::{
    config,
    models::{self, commit::EnrichedCommit, EnrichedUser},
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
use std::{collections::HashMap, fmt, sync::Arc};
use tokio_stream::StreamExt;
use tracing::{info, instrument};

#[derive(Debug, Clone)]
/// Regexes for doing exclude checks
struct ExcludeRegex {
    pub company: Regex,
    pub email: Regex,
}

#[derive(Debug)]
pub struct Output {
    pub user: Option<EnrichedUser>,
    pub membership: bool,
    pub contributions: Vec<Contribution>,
}

#[derive(Debug)]
struct RepoRegex {
    repo: models::Repo,
    companies_exclude: Vec<ExcludeRegex>,
}

impl From<&config::Repo> for RepoRegex {
    fn from(value: &config::Repo) -> Self {
        RepoRegex {
            repo: value.repo.clone(),
            companies_exclude: value
                .companies_exclude
                .iter()
                .map(|company| ExcludeRegex {
                    company: Regex::new(
                        format!(r#"(?i){}"#, regex::escape(company.as_ref())).as_str(),
                    )
                    .unwrap(),
                    email: Regex::new(
                        format!(r#"@(\w\.)*(?i){}(?-i)\."#, regex::escape(company.as_ref()))
                            .as_str(),
                    )
                    .unwrap(),
                })
                .collect(),
        }
    }
}

/// Client for getting different types of GitHub contributions
pub struct GithubContributionCollector {
    client: Arc<octocrab::Octocrab>,
}

impl fmt::Debug for GithubContributionCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubContributionCollector").finish()
    }
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
    #[instrument]
    pub async fn process_contributions(
        &self,
        contributions: impl Iterator<Item = Contribution> + fmt::Debug,
        company_orgs: impl Iterator<Item = impl AsRef<str>> + Clone + fmt::Debug,
        repos: impl Iterator<Item = &config::Repo> + Clone + fmt::Debug,
        user_overrides: impl Iterator<Item = config::UserOverride> + fmt::Debug,
    ) -> Result<Vec<Output>, octocrab::Error> {
        let repos_re = repos.map(|repo| RepoRegex::from(repo));
        let user_overrides_map: HashMap<String, config::UserOverride> = user_overrides
            .map(|user_override| (user_override.login.clone(), user_override))
            .collect();

        let collection = output_stream(
            self.client.clone(),
            contributions,
            company_orgs,
            repos_re,
            user_overrides_map,
        )
        .await
        .collect::<Result<Vec<Output>, octocrab::Error>>()
        .await?;

        Ok(collection)
    }

    /// Return all contributions for a repo.
    #[instrument]
    pub async fn contributions(
        &self,
        repo_org: impl AsRef<str> + fmt::Debug,
        repo: impl AsRef<str> + fmt::Debug,
    ) -> Result<Vec<Contribution>, octocrab::Error> {
        info!(
            "Fetching contributions: {}/{}",
            repo_org.as_ref(),
            repo.as_ref()
        );
        let (issues, reviews, commits) = tokio::join!(
            self.issues(repo_org.as_ref(), repo.as_ref()),
            self.reviews(repo_org.as_ref(), repo.as_ref()),
            self.commits(repo_org.as_ref(), repo.as_ref()),
        );

        let contributions =
            issues?
                .into_iter()
                .map(|issue| Contribution::new(repo_org.as_ref(), repo.as_ref(), issue.into()))
                .chain(reviews?.into_iter().map(|review| {
                    Contribution::new(repo_org.as_ref(), repo.as_ref(), review.into())
                }))
                .chain(commits?.into_iter().map(|commit| {
                    Contribution::new(repo_org.as_ref(), repo.as_ref(), commit.into())
                }))
                .collect();

        Ok(contributions)
    }

    /// Collect all contributions from commits on the default branch associated with this repo.
    #[instrument]
    pub async fn commits(
        &self,
        repo_org: impl AsRef<str> + fmt::Debug,
        repo: impl AsRef<str> + fmt::Debug,
    ) -> Result<Vec<EnrichedCommit>, octocrab::Error> {
        let page: Page<EnrichedCommit> = self
            .client
            .get(
                format!("/repos/{}/{}/commits", repo_org.as_ref(), repo.as_ref()),
                None::<&()>,
            )
            .await?;

        info!(pages = ?page.number_of_pages(), "type" = "commits");

        Ok(self.process_pages(page).await?)
    }

    pub async fn commits_as_contributions(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Contribution>, octocrab::Error> {
        Ok(self
            .commits(repo_org.as_ref(), repo.as_ref())
            .await?
            .into_iter()
            .map(|commit| Contribution::new(repo_org.as_ref(), repo.as_ref(), commit.into()))
            .collect())
    }

    /// Collect all contributions from issues associated with this repo.
    #[instrument]
    pub async fn issues(
        &self,
        repo_org: impl AsRef<str> + fmt::Debug,
        repo: impl AsRef<str> + fmt::Debug,
    ) -> Result<Vec<Issue>, octocrab::Error> {
        let page = self
            .client
            .issues(repo_org.as_ref(), repo.as_ref())
            .list()
            .sort(params::issues::Sort::Created)
            .direction(params::Direction::Descending)
            .send()
            .await?;

        info!(pages = ?page.number_of_pages(), "type" = "issues");

        Ok(self.process_pages(page).await?)
    }

    pub async fn issues_as_contributions(
        &self,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Contribution>, octocrab::Error> {
        Ok(self
            .issues(repo_org.as_ref(), repo.as_ref())
            .await?
            .into_iter()
            .map(|issue| Contribution::new(repo_org.as_ref(), repo.as_ref(), issue.into()))
            .collect())
    }

    /// Collect all contributions reviews associated with this repo.
    #[instrument]
    pub async fn reviews(
        &self,
        repo_org: impl AsRef<str> + fmt::Debug,
        repo: impl AsRef<str> + fmt::Debug,
    ) -> Result<Vec<Review>, octocrab::Error> {
        let pull_handler = self.client.pulls(repo_org.as_ref(), repo.as_ref());
        let page = pull_handler
            .list()
            .sort(params::pulls::Sort::Created)
            .direction(params::Direction::Descending)
            .send()
            .await?;

        info!(pages = ?page.number_of_pages(), "type" = "pull_requests");

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
            .reviews(repo_org.as_ref(), repo.as_ref())
            .await?
            .into_iter()
            .map(|review| Contribution::new(repo_org.as_ref(), repo.as_ref(), review.into()))
            .collect())
    }

    /// Get all the items from the current page until the end
    #[instrument]
    async fn process_pages<T: DeserializeOwned + fmt::Debug>(
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
#[instrument]
fn contributions_by_user(
    contributions: impl Iterator<Item = Contribution> + fmt::Debug,
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
#[instrument]
async fn check_membership(
    login: impl AsRef<str> + fmt::Debug,
    orgs: impl Iterator<Item = OrgHandler<'_>> + fmt::Debug,
) -> Result<bool, octocrab::Error> {
    let mut membership = false;

    for org in orgs {
        membership = membership || org.check_membership(login.as_ref()).await?;
    }

    Ok(membership)
}

/// Enrich user with more data from the GitHub API
#[instrument]
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
#[instrument]
async fn process_pages<T: DeserializeOwned + fmt::Debug>(
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

/// Stream of Pull Request Reviews
#[instrument]
async fn review_stream(
    client: Arc<octocrab::Octocrab>,
    pull_requests: impl Iterator<Item = PullRequest> + fmt::Debug,
    repo_org: impl AsRef<str> + fmt::Debug,
    repo: impl AsRef<str> + fmt::Debug,
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
#[instrument]
async fn output_stream(
    client: Arc<octocrab::Octocrab>,
    contributions: impl Iterator<Item = Contribution> + fmt::Debug,
    company_orgs: impl Iterator<Item = impl AsRef<str>> + Clone + fmt::Debug,
    repos: impl Iterator<Item = RepoRegex> + Clone + fmt::Debug,
    user_overrides: HashMap<String, config::UserOverride>,
) -> impl Stream<Item = Result<Output, octocrab::Error>> {
    try_stream! {
        let orgs = company_orgs
            .map(|org| client.orgs(org.as_ref()));

        for (maybe_user, contributions) in contributions_by_user(contributions) {
            let mut membership = false;
            let mut maybe_company_user = None;
            let mut processed_contributions = contributions;

            if let Some(user) = maybe_user {
                let enriched_user;
                if let Some(override_user) = user_overrides.get(&user.login) {
                    enriched_user = EnrichedUser {
                        inner: user,
                        company: Some(override_user.company.clone()),
                        email: None,
                    };
                } else {
                    enriched_user = enrich_user(&client, user).await?;
                    membership = check_membership(&enriched_user.inner.login, orgs.clone()).await?;
                }

                for config_repo in repos.clone() {
                    processed_contributions = processed_contributions.into_iter().filter(|contribution| {
                        if config_repo.repo == contribution.repo {
                            // if the user matches an org that matches our exclude, don't keep this
                            // contribution
                            return !config_repo.companies_exclude.iter().any(|exclude_re| {
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
                                })
                        }

                        true

                    }).collect();
                }

                maybe_company_user = Some(enriched_user);
            }

            // don't yield an output if there are no contributions if they got excluded
            if processed_contributions.len() <= 0 {
                continue
            }

            yield Output {
                user: maybe_company_user,
                membership,
                contributions: processed_contributions,
            };
        }
    }
}

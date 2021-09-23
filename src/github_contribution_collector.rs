use crate::{
    config,
    models::{self, commit::EnrichedCommit, EnrichedUser},
    Contribution,
};
use async_recursion::async_recursion;
use async_stream::try_stream;
use chrono::{offset::TimeZone, DateTime};
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
use std::{collections::HashMap, fmt, marker::Send, sync::Arc};
use tokio_stream::StreamExt;
use tracing::{info, instrument};

const MAX_TRIES: usize = 5;

/// Common Parameters for the GitHub API
#[derive(Clone, Debug)]
pub struct Params<TzA: TimeZone, TzB: TimeZone>
where
    TzA::Offset: fmt::Display,
    TzB::Offset: fmt::Display,
{
    pub since: Option<DateTime<TzA>>,
    pub until: Option<DateTime<TzB>>,
}

impl<TzA: TimeZone, TzB: TimeZone> Params<TzA, TzB>
where
    TzA::Offset: fmt::Display,
    TzB::Offset: fmt::Display,
{
    /// convert into the data structure reqwest uses for parameters
    fn to_params(&self) -> Option<Vec<(&str, String)>> {
        let mut params_vec = vec![];
        if let Some(since) = self.since.as_ref() {
            params_vec.push(("since", since.to_rfc3339()));
        }
        if let Some(after) = self.until.as_ref() {
            params_vec.push(("until", after.to_rfc3339()));
        }

        if params_vec.is_empty() {
            None
        } else {
            Some(params_vec)
        }
    }
}

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
    #[instrument(skip(self, contributions))]
    pub async fn process_contributions<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
        &self,
        contributions: impl Iterator<Item = Contribution> + fmt::Debug,
        company_orgs: impl Iterator<Item = impl AsRef<str>> + Clone + fmt::Debug,
        repos: impl Iterator<Item = &config::Repo> + Clone + fmt::Debug,
        user_overrides: impl Iterator<Item = config::UserOverride> + fmt::Debug,
        users_exclude: impl Iterator<Item = impl AsRef<str>> + Clone + fmt::Debug,
        params: Params<TzA, TzB>,
    ) -> Result<Vec<Output>, octocrab::Error>
    where
        TzA::Offset: fmt::Display,
        TzB::Offset: fmt::Display,
    {
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
            users_exclude,
            params,
        )
        .await
        .collect::<Result<Vec<Output>, octocrab::Error>>()
        .await?;

        Ok(collection)
    }

    /// Return all contributions for a repo.
    #[instrument(skip(self))]
    pub async fn contributions<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
        &self,
        repo: &models::Repo,
        params: &Params<TzA, TzB>,
    ) -> Result<Vec<Contribution>, octocrab::Error>
    where
        TzA::Offset: fmt::Display,
        TzB::Offset: fmt::Display,
    {
        info!("Fetching contributions: {}", repo);
        let (issues, reviews, commits) = tokio::join!(
            self.issues(repo, params),
            self.reviews(repo),
            self.commits(repo, params),
        );

        let contributions = issues?
            .into_iter()
            .map(|issue| Contribution::new(&repo.org, &repo.name, issue.into()))
            .chain(
                reviews?
                    .into_iter()
                    .map(|review| Contribution::new(&repo.org, &repo.name, review.into())),
            )
            .chain(
                commits?
                    .into_iter()
                    .map(|commit| Contribution::new(&repo.org, &repo.name, commit.into())),
            )
            .collect();

        Ok(contributions)
    }

    /// Collect all contributions from commits on the default branch associated with this repo.
    #[instrument(skip(self))]
    pub async fn commits<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
        &self,
        repo: &models::Repo,
        params: &Params<TzA, TzB>,
    ) -> Result<Vec<EnrichedCommit>, octocrab::Error>
    where
        TzA::Offset: fmt::Display,
        TzB::Offset: fmt::Display,
    {
        match commit_page(self.client.clone(), repo, params).await? {
            Some(page) => {
                info!(pages = ?page.number_of_pages(), "type" = "commits");
                Ok(process_pages(&self.client, page).await?)
            }
            None => Ok(vec![]),
        }
    }

    /// Collect all contributions from issues associated with this repo.
    #[instrument(skip(self))]
    pub async fn issues<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
        &self,
        repo: &models::Repo,
        params: &Params<TzA, TzB>,
    ) -> Result<Vec<Issue>, octocrab::Error>
    where
        TzA::Offset: fmt::Display,
        TzB::Offset: fmt::Display,
    {
        match issues_page(self.client.clone(), repo, params).await? {
            Some(page) => {
                info!(pages = ?page.number_of_pages(), "type" = "issues");

                Ok(process_pages(&self.client, page).await?)
            }
            None => Ok(vec![]),
        }
    }

    /// Collect all contributions reviews associated with this repo.
    #[instrument(skip(self))]
    pub async fn reviews(&self, repo: &models::Repo) -> Result<Vec<Review>, octocrab::Error> {
        match pull_request_page(self.client.clone(), repo).await? {
            Some(page) => {
                info!(pages = ?page.number_of_pages(), "type" = "pull_requests");
                let pull_requests = process_pages(&self.client, page).await?;

                let reviews =
                    review_stream(self.client.clone(), pull_requests.into_iter(), repo.clone())
                        .await
                        .collect::<Result<Vec<Vec<Review>>, octocrab::Error>>()
                        .await?
                        .into_iter()
                        .flatten()
                        .collect();

                Ok(reviews)
            }
            None => Ok(vec![]),
        }
    }
}

#[async_recursion]
async fn retry_get_page<T: 'async_recursion + DeserializeOwned + fmt::Debug + Send>(
    client: &octocrab::Octocrab,
    url: &Option<url::Url>,
    tries_left: usize,
) -> octocrab::Result<Option<Page<T>>> {
    let result = client.get_page::<T>(url).await;

    if result.is_err() && tries_left >= 2 {
        retry_get_page(client, url, tries_left - 1).await
    } else {
        result
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
#[instrument(skip(client, user))]
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
#[instrument(skip(client, page))]
async fn process_pages<T: DeserializeOwned + fmt::Debug + Send>(
    client: &octocrab::Octocrab,
    mut page: Page<T>,
) -> Result<Vec<T>, octocrab::Error> {
    let mut items = Vec::new();

    loop {
        for item in page.take_items() {
            items.push(item);
        }

        let next_page_option = retry_get_page(&client, &page.next, MAX_TRIES).await?;
        if let Some(next_page) = next_page_option {
            page = next_page;
        } else {
            break;
        }
    }

    Ok(items)
}

/// Stream of Pull Request Reviews
#[instrument(skip(client, pull_requests))]
async fn review_stream(
    client: Arc<octocrab::Octocrab>,
    pull_requests: impl Iterator<Item = PullRequest>,
    repo: models::Repo,
) -> impl Stream<Item = Result<Vec<Review>, octocrab::Error>> {
    try_stream! {
        for pull_request in pull_requests {
            let pull_handler = &client.pulls(&repo.org, &repo.name);
            let page = pull_handler.list_reviews(pull_request.number).await?;
            let items = process_pages(&client, page).await?;

            yield items;
        }
    }
}

/// Build an output stream
#[instrument(skip(client))]
async fn output_stream<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
    client: Arc<octocrab::Octocrab>,
    contributions: impl Iterator<Item = Contribution> + fmt::Debug,
    company_orgs: impl Iterator<Item = impl AsRef<str>> + Clone + fmt::Debug,
    repos: impl Iterator<Item = RepoRegex> + Clone + fmt::Debug,
    user_overrides: HashMap<String, config::UserOverride>,
    users_exclude: impl Iterator<Item = impl AsRef<str>> + Clone + fmt::Debug,
    params: Params<TzA, TzB>,
) -> impl Stream<Item = Result<Output, octocrab::Error>>
where
    TzA::Offset: fmt::Display,
    TzB::Offset: fmt::Display,
{
    try_stream! {
        let orgs = company_orgs.clone()
            .map(|org| client.orgs(org.as_ref()));

        for (maybe_user, contributions) in contributions_by_user(contributions) {
            let mut membership = false;
            let mut maybe_company_user = None;
            let mut processed_contributions = contributions;

            if let Some(user) = maybe_user {
                if users_exclude.clone().find(|login| user.login == login.as_ref()).is_some() {
                    continue
                }

                let enriched_user;
                if let Some(override_user) = user_overrides.get(&user.login) {
                    enriched_user = EnrichedUser {
                        inner: user,
                        company: Some(override_user.company.clone()),
                        email: None,
                    };
                    membership = company_orgs.clone().find(|org| org.as_ref() == override_user.company).is_some();
                } else {
                    enriched_user = enrich_user(&client, user).await?;
                    membership = check_membership(&enriched_user.inner.login, orgs.clone()).await?;
                }

                for config_repo in repos.clone() {
                    processed_contributions = processed_contributions.into_iter().filter(|contribution| {
                        // filter out contributions out of the specified range
                        if let Some(contribution_time) = contribution.created_at() {
                            if let Some(since) = params.since.as_ref() {
                                if contribution_time < since.with_timezone(&contribution_time.timezone()) {
                                    return false;
                                }
                            }
                            if let Some(until) = params.until.as_ref() {
                                if contribution_time >= until.with_timezone(&contribution_time.timezone()) {
                                    return false;
                                }
                            }
                        }

                        // filter out users explicitly called to be excluded
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

#[instrument(skip(client))]
async fn issues_page<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
    client: Arc<octocrab::Octocrab>,
    repo: &models::Repo,
    params: &Params<TzA, TzB>,
) -> Result<Option<Page<Issue>>, octocrab::Error>
where
    TzA::Offset: fmt::Display,
    TzB::Offset: fmt::Display,
{
    match client
        .get(
            format!("/repos/{}/issues", repo),
            params.to_params().as_ref(),
        )
        .await
    {
        Ok(page) => Ok(Some(page)),
        Err(err) => {
            match err {
                // for cases when the the issues page is turned off
                octocrab::Error::GitHub { source, backtrace } => {
                    if source.documentation_url
                        == "https://docs.github.com/rest/reference/issues#list-repository-issues"
                    {
                        eprintln!("Could not fetch issues: {}", &repo);
                        Ok(None)
                    } else {
                        Err(octocrab::Error::GitHub { source, backtrace })
                    }
                }
                _ => Err(err),
            }
        }
    }
}

async fn pull_request_page(
    client: Arc<octocrab::Octocrab>,
    repo: &models::Repo,
) -> Result<Option<Page<PullRequest>>, octocrab::Error> {
    let pull_handler = client.pulls(&repo.org, &repo.name);
    match pull_handler
        .list()
        .sort(params::pulls::Sort::Created)
        .direction(params::Direction::Descending)
        .send()
        .await
    {
        Ok(page) => Ok(Some(page)),
        Err(err) => {
            match err {
                // for cases when the the issues page is turned off
                octocrab::Error::GitHub { source, backtrace } => {
                    if source.documentation_url
                        == "https://docs.github.com/rest/reference/pulls#list-pull-requests"
                    {
                        eprintln!("Could not fetch pull requests: {}", &repo);
                        Ok(None)
                    } else {
                        Err(octocrab::Error::GitHub { source, backtrace })
                    }
                }
                _ => Err(err),
            }
        }
    }
}

async fn commit_page<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
    client: Arc<octocrab::Octocrab>,
    repo: &models::Repo,
    params: &Params<TzA, TzB>,
) -> Result<Option<Page<EnrichedCommit>>, octocrab::Error>
where
    TzA::Offset: fmt::Display,
    TzB::Offset: fmt::Display,
{
    match client
        .get(
            format!("/repos/{}/{}/commits", repo.org, repo.name),
            params.to_params().as_ref(),
        )
        .await
    {
        Ok(page) => Ok(Some(page)),
        Err(err) => {
            match err {
                // for cases when the the commits page is turned off
                octocrab::Error::GitHub { source, backtrace } => {
                    if source.documentation_url
                        == "https://docs.github.com/rest/reference/repos#list-commits"
                    {
                        eprintln!("Could not fetch commits: {}", &repo);
                        Ok(None)
                    } else {
                        Err(octocrab::Error::GitHub { source, backtrace })
                    }
                }
                _ => Err(err),
            }
        }
    }
}

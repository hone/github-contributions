use crate::{
    models::{commit::EnrichedCommit, EnrichedUser},
    Contribution,
};
use octocrab::{
    models::{issues::Issue, pulls::Review, User},
    orgs::OrgHandler,
    params, Page,
};
use regex::Regex;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use tokio_stream::StreamExt;

#[derive(Debug)]
pub struct Output {
    pub user: Option<EnrichedUser>,
    pub membership: bool,
    pub exclude: bool,
    pub contributions: Vec<Contribution>,
}

pub struct GithubContributionCollector {
    client: octocrab::Octocrab,
}

#[derive(Debug, Clone)]
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
            client: client.build()?,
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
        let exclude_orgs_re = exclude_orgs.iter().map(|org| ExcludeRegex {
            company: Regex::new(format!(r#"{}"#, regex::escape(org.as_ref())).as_str()).unwrap(),
            email: Regex::new(format!(r#"@(\w\.)*{}\."#, regex::escape(org.as_ref())).as_str())
                .unwrap(),
        });
        let output_stream = tokio_stream::iter(contributions_by_user(contributions))
            .map(|(maybe_user, contributions)| {
                // create copies so they can be moved inside the async block
                let cloned_client = self.client.clone();
                let orgs = company_orgs
                    .iter()
                    .map(|org| self.client.orgs(org.as_ref()));
                let mut exclude_orgs_re_clone = exclude_orgs_re.clone();

                async move {
                    let mut membership = false;
                    let mut exclude = false;
                    let mut maybe_company_user = None;

                    if let Some(user) = maybe_user {
                        let enriched_user = enrich_user(&cloned_client, user).await?;
                        membership = check_membership(&enriched_user.inner.login, orgs).await?;
                        exclude = exclude_orgs_re_clone.any(|exclude_re| {
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

                    Result::<_, octocrab::Error>::Ok(Output {
                        user: maybe_company_user,
                        membership,
                        exclude,
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

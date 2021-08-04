use octocrab::{
    models::{issues::Issue, IssueId, User},
    Page,
};
use serde::Deserialize;
use std::collections::HashMap;
use tokio_stream::StreamExt;

pub enum Contribution {
    IssueId(IssueId),
}

#[derive(Deserialize)]
pub struct UserWithCompanyInfo {
    #[serde(flatten)]
    pub inner: User,
    pub company: Option<String>,
    pub email: Option<String>,
}

pub struct Output {
    pub user: UserWithCompanyInfo,
    pub membership: bool,
    pub contribution_count: usize,
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

    pub async fn issues(
        &self,
        company_org: impl AsRef<str>,
        repo_org: impl AsRef<str>,
        repo: impl AsRef<str>,
    ) -> Result<Vec<Output>, octocrab::Error> {
        let mut contributions: HashMap<User, Vec<Contribution>> = HashMap::new();

        let issues_page = self
            .client
            .issues(repo_org.as_ref(), repo.as_ref())
            .list()
            .per_page(5)
            .send()
            .await?;
        process_page(&issues_page, &mut contributions);

        let mut next_page = issues_page.next;
        while let Some(page) = self.client.get_page::<Issue>(&next_page).await? {
            process_page(&page, &mut contributions);
            next_page = page.next;
        }

        let stream = tokio_stream::iter(&contributions);
        tokio::pin!(stream);
        let output_stream = stream
            .map(|(user, contributions)| {
                // create copies so they can be moved inside the async block
                let cloned_client = self.client.clone();
                let heroku_org = self.client.orgs(company_org.as_ref());

                async move {
                    let company_user: UserWithCompanyInfo = cloned_client
                        .get(format!("/users/{}", user.login), None::<&()>)
                        .await?;

                    let membership = heroku_org
                        .check_membership(&company_user.inner.login)
                        .await?;

                    Result::<_, octocrab::Error>::Ok(Output {
                        user: company_user,
                        membership,
                        contribution_count: contributions.len(),
                    })
                }
            })
            .collect::<Vec<_>>()
            .await;

        Ok(futures::future::try_join_all(output_stream).await?)
    }
}

fn process_page(page: &Page<Issue>, contributions: &mut HashMap<User, Vec<Contribution>>) {
    for item in page {
        let user_contributions = contributions.entry(item.user.clone()).or_insert(Vec::new());
        (*user_contributions).push(Contribution::IssueId(item.id));
    }
}

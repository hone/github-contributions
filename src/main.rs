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

struct Output {
    pub user: UserWithCompanyInfo,
    pub membership: bool,
    pub contribution_count: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let github_token = std::env::var("GITHUB_TOKEN").unwrap_or_else(|_| {
        eprintln!("Please provide a GITHUB_TOKEN");

        std::process::exit(1);
    });

    let client = octocrab::OctocrabBuilder::new()
        .personal_token(github_token)
        .build()?;
    let issues_page = client
        .issues("buildpacks", "pack")
        .list()
        .per_page(5)
        .send()
        .await?;
    let mut contributions: HashMap<User, Vec<Contribution>> = HashMap::new();

    process_page(&issues_page, &mut contributions);

    let mut next_page = issues_page.next;
    while let Some(page) = client.get_page::<Issue>(&next_page).await? {
        process_page(&page, &mut contributions);
        next_page = page.next;
    }

    let heroku_org = client.orgs("heroku");

    println!(
        "{0: <20} {1: <10} {2: <10}",
        "handle", "salesforce", "contributions"
    );
    let stream = tokio_stream::iter(contributions);
    tokio::pin!(stream);
    let output: Vec<()> = stream
        .map(|(user, contributions)| async move {
            let company_user: UserWithCompanyInfo = client
                .get(format!("/users/{}", user.login), None::<&()>)
                .await
                .unwrap();
        })
        .collect()
        .await;
    for (user, contributions) in contributions.iter() {
        let company_user: UserWithCompanyInfo = client
            .get(format!("/users/{}", user.login), None::<&()>)
            .await?;
        println!(
            "{0: <20} {1: <10} {2: <10}",
            company_user.inner.login,
            heroku_org
                .check_membership(&company_user.inner.login)
                .await?,
            contributions.len(),
        );
    }

    Ok(())
}

fn process_page(page: &Page<Issue>, contributions: &mut HashMap<User, Vec<Contribution>>) {
    for item in page {
        let user_contributions = contributions.entry(item.user.clone()).or_insert(Vec::new());
        (*user_contributions).push(Contribution::IssueId(item.id));
    }
}

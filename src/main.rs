use chrono::offset::{TimeZone, Utc};
use github_contributions::{
    config::Config, contribution::GithubContribution, github_contribution_collector::Params,
    Contribution, GithubContributionCollector,
};
use std::{fmt, sync::Arc};

use async_stream::try_stream;
use futures::{future::join_all, Stream};
use tokio_stream::StreamExt;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

async fn contributions_stream<TzA: TimeZone + fmt::Debug, TzB: TimeZone + fmt::Debug>(
    client: Arc<GithubContributionCollector>,
    repos: impl Iterator<Item = &github_contributions::config::Repo>,
    params: Params<TzA, TzB>,
) -> impl Stream<Item = Result<Vec<Contribution>, octocrab::Error>>
where
    TzA::Offset: fmt::Display,
    TzB::Offset: fmt::Display,
{
    try_stream! {
        let mut tasks = vec![];
        // queue up all tasks first
        for repo in repos {
            tasks.push(client.contributions(&repo.repo.org, &repo.repo.name, &params));
        }
        for result in join_all(tasks).await {
            let contributions = result?;
            yield contributions;
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(EnvFilter::from_default_env())
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let args = std::env::args().collect::<Vec<String>>();
    let github_token = std::env::var("GITHUB_TOKEN").unwrap_or_else(|_| {
        eprintln!("Please provide a GITHUB_TOKEN");

        std::process::exit(1);
    });
    let config: Config = toml::from_str(&std::fs::read_to_string(&args[1])?)?;
    let client = Arc::new(GithubContributionCollector::new(Some(github_token))?);
    let params = Params {
        since: Some(Utc.ymd(2021, 5, 1).and_hms(0, 0, 0)),
        until: Some(Utc.ymd(2021, 8, 1).and_hms(0, 0, 0)),
    };
    let contributions = contributions_stream(client.clone(), config.repos.iter(), params.clone())
        .await
        .collect::<Result<Vec<Vec<Contribution>>, octocrab::Error>>()
        .await?
        .into_iter()
        .flatten();
    let mut outputs = client
        .process_contributions(
            contributions.into_iter(),
            config.company_organizations.iter(),
            config.repos.iter(),
            config.user_overrides.into_iter(),
            params.clone(),
        )
        .await?;

    outputs.retain(|output| {
        !output.membership
            && output.contributions.len() > 0
            && output
                .user
                .as_ref()
                .map(|u| u.inner.login != "dependabot[bot]")
                .unwrap_or(false)
    });
    outputs.sort_by(|a, b| {
        b.contributions
            .len()
            .partial_cmp(&a.contributions.len())
            .unwrap()
    });

    println!(
        "{0: <20} {1: <10} {2: <10} {3: <10} {4: <10} {5: <10}",
        "handle", "salesforce", "issues", "reviews", "commits", "all"
    );
    for output in outputs.iter() {
        let mut issues_count = 0;
        let mut reviews_count = 0;
        let mut commits_count = 0;

        for contribution in output.contributions.iter() {
            match contribution.contribution {
                GithubContribution::Issue(_) => issues_count += 1,
                GithubContribution::Review(_) => reviews_count += 1,
                GithubContribution::Commit(_) => commits_count += 1,
            }
        }
        println!(
            "{0: <20} {1: <10} {2: <10} {3: <10} {4: <10} {5: <10}",
            output
                .user
                .as_ref()
                .map(|u| u.inner.login.as_str())
                .unwrap_or("None"),
            output.membership,
            issues_count,
            reviews_count,
            commits_count,
            output.contributions.len(),
        );
    }
    println!(
        "Total Contributions: {}",
        outputs
            .iter()
            .fold(0, |sum, output| sum + output.contributions.len())
    );

    Ok(())
}

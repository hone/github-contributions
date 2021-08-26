use async_stream::try_stream;
use chrono::offset::TimeZone;
use futures::{future::join_all, Stream};
use github_contributions::{
    cli, config::Config, contribution::GithubContribution, github_contribution_collector::Params,
    models::Repo, Contribution, GithubContributionCollector,
};
use std::{collections::HashMap, fmt, sync::Arc};
use structopt::StructOpt;
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

    let github_token = std::env::var("GITHUB_TOKEN").unwrap_or_else(|_| {
        eprintln!("Please provide a GITHUB_TOKEN");

        std::process::exit(1);
    });
    let args = cli::Opt::from_args();
    let config: Config = toml::from_str(&std::fs::read_to_string(&args.config)?)?;
    let client = Arc::new(GithubContributionCollector::new(Some(github_token))?);
    let params = Params {
        since: args.start,
        until: args.end,
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
            config.users_exclude.iter(),
            params.clone(),
        )
        .await?;

    outputs.retain(|output| !output.membership);
    outputs.sort_by(|a, b| {
        b.contributions
            .len()
            .partial_cmp(&a.contributions.len())
            .unwrap()
    });

    println!(
        "{0: <40} {1: <10} {2: <10} {3: <10} {4: <10}",
        "handle", "issues", "reviews", "commits", "all"
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
            "{0: <40} {1: <10} {2: <10} {3: <10} {4: <10}",
            output
                .user
                .as_ref()
                .map(|u| u.inner.login.as_str())
                .unwrap_or("None"),
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

    let mut per_repo: HashMap<Repo, Vec<Contribution>> = HashMap::new();
    for contribution in outputs
        .iter()
        .flat_map(|output| output.contributions.clone())
    {
        let value = per_repo
            .entry(contribution.repo.clone())
            .or_insert(Vec::new());
        (*value).push(contribution.clone());
    }

    println!("--");

    println!(
        "{0: <40} {1: <10} {2: <10} {3: <10} {4: <10}",
        "repo", "issues", "reviews", "commits", "all"
    );
    for (repo, contributions) in per_repo.iter() {
        let mut issues_count = 0;
        let mut reviews_count = 0;
        let mut commits_count = 0;

        for contribution in contributions.iter() {
            match contribution.contribution {
                GithubContribution::Issue(_) => issues_count += 1,
                GithubContribution::Review(_) => reviews_count += 1,
                GithubContribution::Commit(_) => commits_count += 1,
            }
        }
        println!(
            "{0: <40} {1: <10} {2: <10} {3: <10} {4: <10}",
            format!("{}/{}", repo.org, repo.name),
            issues_count,
            reviews_count,
            commits_count,
            contributions.len(),
        );
    }
    println!(
        "Total Contributions: {}",
        per_repo
            .iter()
            .fold(0, |sum, (_, contributions)| sum + contributions.len())
    );

    Ok(())
}

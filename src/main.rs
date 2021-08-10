use chrono::offset::{TimeZone, Utc};
use github_contributions::{Contribution, GithubContributionCollector};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args().collect::<Vec<String>>();
    let github_token = std::env::var("GITHUB_TOKEN").unwrap_or_else(|_| {
        eprintln!("Please provide a GITHUB_TOKEN");

        std::process::exit(1);
    });
    let org = &args[1];
    let repo = &args[2];
    let client = GithubContributionCollector::new(Some(github_token))?;
    let issues = client
        .issues(&org, &repo)
        .await?
        .into_iter()
        .map(|issue| issue.into());
    let reviews = client
        .reviews(&org, &repo)
        .await?
        .into_iter()
        .map(|review| review.into());
    let commits = client
        .commits(&org, &repo)
        .await?
        .into_iter()
        .map(|commit| commit.into());
    let mut outputs = client
        .process_contributions(
            issues
                .into_iter()
                .chain(reviews.into_iter())
                .chain(commits.into_iter()),
            vec!["heroku", "salesforce", "forcedotcom"],
            vec!["vmware", "pivotal"],
        )
        .await?;

    for output in outputs.iter_mut() {
        output.contributions.retain(|contribution| {
            contribution.created_at() >= Some(Utc.ymd(2021, 5, 1).and_hms(0, 0, 0))
        });
    }
    outputs.retain(|output| {
        !output.membership
            && !output.exclude
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
            match contribution {
                Contribution::Issue(_) => issues_count += 1,
                Contribution::Review(_) => reviews_count += 1,
                Contribution::Commit(_) => commits_count += 1,
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

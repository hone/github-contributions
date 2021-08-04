use github_contributions::GithubContributionCollector;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let github_token = std::env::var("GITHUB_TOKEN").unwrap_or_else(|_| {
        eprintln!("Please provide a GITHUB_TOKEN");

        std::process::exit(1);
    });
    let client = GithubContributionCollector::new(Some(github_token))?;
    let mut outputs = client.issues("heroku", "buildpacks", "pack").await?;

    println!(
        "{0: <20} {1: <10} {2: <10}",
        "handle", "salesforce", "contributions"
    );
    outputs.sort_by(|a, b| {
        b.contribution_count
            .partial_cmp(&a.contribution_count)
            .unwrap()
    });
    for output in outputs.iter().filter(|output| !output.membership) {
        println!(
            "{0: <20} {1: <10} {2: <10}",
            output.user.inner.login, output.membership, output.contribution_count
        );
    }

    Ok(())
}

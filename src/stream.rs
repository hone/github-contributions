use async_stream::stream;
use futures::{Future, Stream, StreamExt};
use octocrab::{
    models::pulls::{PullRequest, Review},
    Page,
};
use serde::de::DeserializeOwned;
use std::sync::Arc;

async fn review_page_stream(
    client: octocrab::Octocrab,
    pull_requests: impl IntoIterator<Item = PullRequest>,
    repo_org: String,
    repo: String,
) -> impl Stream<Item = impl Future<Output = Result<Page<Review>, octocrab::Error>>> {
    futures::stream::iter(pull_requests).map(move |pull_request| {
        let pull_handler = octocrab::instance().pulls(repo_org.clone(), repo.clone());
        pull_handler.list_reviews(pull_request.number)
    })
}

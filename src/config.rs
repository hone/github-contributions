use crate::models;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
/// Config file for contribution collector
/// # Example
/// company_organizations = [
///   "heroku",
///   "forcedotcom"
/// ],
/// [repo]
pub struct Config {
    pub company_organizations: Vec<String>,
    #[serde(default)]
    pub repos: Vec<Repo>,
    #[serde(default)]
    pub orgs: Vec<Org>,
    #[serde(default)]
    pub user_overrides: Vec<UserOverride>,
    #[serde(default)]
    pub users_exclude: Vec<String>,
}

#[derive(Clone, Deserialize, Debug)]
pub struct Repo {
    #[serde(flatten)]
    pub repo: models::Repo,
    #[serde(default)]
    pub companies_exclude: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Org {
    pub name: String,
    pub companies_exclude: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserOverride {
    pub login: String,
    pub company: String,
}

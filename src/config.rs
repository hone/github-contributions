use serde::Deserialize;

#[derive(Deserialize)]
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
}

#[derive(Deserialize)]
pub struct Repo {
    pub org: String,
    pub name: String,
    #[serde(default)]
    pub companies_exclude: Vec<String>,
}

#[derive(Deserialize)]
pub struct Org {
    pub name: String,
    pub companise_exclude: Vec<String>,
}

#[derive(Deserialize)]
pub struct UserOverride {
    pub login: String,
    pub company: String,
}

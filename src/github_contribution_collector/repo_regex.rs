use crate::{config, models, Contribution};
use regex::Regex;

#[derive(Debug, Clone)]
/// Regexes for doing exclude checks
pub(super) struct ExcludeRegex {
    pub company: Regex,
    pub email: Regex,
}

#[derive(Debug)]
/// Repo with Exclude Regex for deciding if a contribution should be counted
pub(super) struct RepoRegex {
    pub repo: models::Repo,
    pub companies_exclude: Vec<ExcludeRegex>,
}

impl From<&config::Repo> for RepoRegex {
    fn from(value: &config::Repo) -> Self {
        RepoRegex {
            repo: value.repo.clone(),
            companies_exclude: value
                .companies_exclude
                .iter()
                .map(|company| ExcludeRegex {
                    company: Regex::new(
                        format!(r#"(?i){}"#, regex::escape(company.as_ref())).as_str(),
                    )
                    .unwrap(),
                    email: Regex::new(
                        format!(r#"@(\w\.)*(?i){}(?-i)\."#, regex::escape(company.as_ref()))
                            .as_str(),
                    )
                    .unwrap(),
                })
                .collect(),
        }
    }
}

impl RepoRegex {
    /// Determine if the contribution should be excluded
    pub fn exclude(&self, contribution: Contribution, user: models::EnrichedUser) -> bool {
        // filter out users explicitly called to be excluded
        if self.repo == contribution.repo {
            // if the user matches an org that matches our exclude, don't keep this
            // contribution
            return !self.companies_exclude.iter().any(|exclude_re| {
                user.company
                    .as_ref()
                    .map(|company| exclude_re.company.is_match(&company))
                    .unwrap_or(false)
                    || user
                        .email
                        .as_ref()
                        .map(|email| exclude_re.email.is_match(email))
                        .unwrap_or(false)
            });
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {}

use chrono::{offset::Utc, DateTime};
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "ghc", about = "GitHub Contributions")]
/// Options for the CLI
pub struct Opt {
    #[structopt(
        parse(from_os_str),
        default_value("config.toml"),
        help = "path to configuration file"
    )]
    pub config: PathBuf,
    #[structopt(
        short,
        long,
        help = "contribution start time in rfc3339 format, ex: 2021-05-01T00:00:00-00:00"
    )]
    pub start: Option<DateTime<Utc>>,
    #[structopt(
        short,
        long,
        help = "contribution end time in rfc3339 format, ex: 2021-08-01T00:00:00-00:00"
    )]
    pub end: Option<DateTime<Utc>>,
}

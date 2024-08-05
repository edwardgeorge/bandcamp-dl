use clap::Parser;
use std::path::PathBuf;

use crate::{api::Format, cookies::Browser};

#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Options {
    #[arg(short, long, default_value_t = Default::default())]
    pub browser: Browser,
    #[arg(short, long, default_value_t = Default::default())]
    pub format: Format,
    #[arg(short = 'd', long = "destination", value_name = "DIRECTORY")]
    pub target: PathBuf,
}

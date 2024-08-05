use clap::Parser;
use dlcommon::cookies::Browser;
use std::path::PathBuf;

use crate::Format;

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

use std::{
    error::Error,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

mod api;
mod cli;

use clap::Parser;
use cli::Options;
use dlcommon::{
    cookies::get_cookies,
    http::{get_client, FileDownload},
    operation::{Operation, Source},
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::time::Duration;
use tokio::sync::Semaphore;

use api::*;

#[tokio::main]
async fn main() -> ExitCode {
    let opts = cli::Options::parse();
    if let Err(e) = run(opts).await {
        eprintln!("Error: {e}");
        ExitCode::FAILURE
    } else {
        println!("Finished!");
        ExitCode::SUCCESS
    }
}

#[inline]
fn spin_style() -> ProgressStyle {
    ProgressStyle::default_spinner().tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏◇")
}

#[inline]
fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
        .unwrap()
        .progress_chars("█▓▒░▫")
}

struct BandcampSource {
    client: Arc<Client>,
    semaphore: Arc<Semaphore>,
    mult: Arc<MultiProgress>,
    items: Vec<(Item, String)>,
    format: Format,
    dest: PathBuf,
}

impl Source for BandcampSource {
    fn apply_to_downloads<F, R>(
        self,
        f: F,
    ) -> impl std::future::Future<Output = Result<(), Box<dyn Error>>>
    where
        F: Fn(dlcommon::http::FileDownload) -> R,
        R: std::future::Future<Output = Result<(), Box<dyn Error>>>,
    {
        async move {
            for (item, url) in self.items {
                let s = self.semaphore.clone().acquire_owned().await?;
                let spin = self.mult.add(
                    //&main_bar,
                    ProgressBar::new_spinner()
                        .with_style(spin_style())
                        .with_message(item.display().to_string()),
                );
                spin.enable_steady_tick(Duration::from_millis(100));
                let u = get_download_link(&self.client, &url, self.format)
                    .await
                    .map_err(|e| format!("Attempting to get download link: {e}"))?;
                spin.finish();
                self.mult.remove(&spin);
                let dl = FileDownload::builder()
                    .title(item.display())
                    .url(u)
                    .target(&self.dest)
                    .preflight_head(false)
                    .filename_use_content_disposition(dlcommon::http::UsagePref::Require)
                    .build()?;
                drop(s);
                f(dl).await?;
            }
            Ok(())
        }
    }
    fn num_downloads(&self) -> u64 {
        self.items.len() as u64
    }
}

async fn download_all(
    client: Arc<Client>,
    items: Vec<(Item, String)>,
    format: Format,
    dest: &Path,
) -> Result<(), Box<dyn Error>> {
    let sem = Arc::new(Semaphore::new(4));
    let mult = Arc::new(MultiProgress::new());
    let src = BandcampSource {
        client: client.clone(),
        dest: dest.to_owned(),
        semaphore: sem.clone(),
        mult: mult.clone(),
        items,
        format,
    };
    Operation::builder()
        .client(client.clone())
        .with_semaphore(sem)
        .multiprogress(mult)
        .build()?
        .run(src)
        .await?;
    Ok(())
}

macro_rules! try_spin_inner {
    ($v:ident, $s:ident, (), (), ()) => {};
    ($v:ident, $s:ident, $x:ident, $y:ident, $t:expr) => {
        let $x = $v;
        let $y = $s;
        $t;
    };
}

macro_rules! try_spin {
    ($msg:expr, $a:expr; |$x:ident, $y:ident| $t:tt) => {{
        let s = ProgressBar::new_spinner()
            .with_style(spin_style())
            .with_message($msg);
        s.enable_steady_tick(Duration::from_millis(100));
        let r = $a;
        match &r {
            Ok(v) => {
                s.finish();
                try_spin_inner!(v, s, $x, $y, $t);
            }
            Err(_) => s.finish(),
        }
        r
    }};
}

async fn run(options: Options) -> Result<(), Box<dyn Error>> {
    let c = try_spin!(format!("getting cookies from {}", options.browser), get_cookies(options.browser); |_v, s| { s.set_message("got cookies"); })?;
    let client = get_client(Some(Arc::new(c)))?;
    let summary = try_spin!("checking credentials", collection_summary(&client).await; |summary, s| {
        s.set_message(format!(
            "logged in as {} ({})",
            summary.collection_summary.username, summary.collection_summary.fan_id,
        ));
    })?;
    let profile = try_spin!("retrieving profile", user_profile(&client, &summary.collection_summary.url).await; |p, s| {
        s.set_message(format!(
            "collection_count: {}",
            p.collection_count
        ));
    })?;
    let progress = ProgressBar::new(profile.collection_count as u64)
        .with_style(bar_style())
        .with_message("retrieving latest collection");
    progress.set_position(profile.collection_data.batch_size as u64);
    let items = list_remaining_collection(
        &client,
        summary.fan_id,
        &profile,
        Some(|t| progress.set_position(t as u64)),
    )
    .await?
    .into_iter()
    .filter(|(it, _)| {
        if !it.download_available() {
            eprintln!("⚠ '{}' has no downloads available.", it.display());
            false
        } else {
            true
        }
    })
    .collect::<Vec<_>>();

    download_all(Arc::new(client), items, options.format, &options.target).await?;
    Ok(())
}

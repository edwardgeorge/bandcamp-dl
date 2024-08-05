use std::{
    error::Error,
    path::Path,
    process::ExitCode,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

mod api;
mod cli;
mod cookies;
mod file;
mod http;

use clap::Parser;
use cli::Options;
use http::Outcome;
use indicatif::{DecimalBytes, MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::time::Duration;
use tokio::{
    spawn,
    sync::{Mutex, Semaphore},
    time::sleep,
};

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

#[inline]
fn dl_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "[{elapsed_precise}] {bar:40.cyan/blue} {decimal_bytes:>12}/{decimal_total_bytes:12} {msg}",
    )
    .unwrap()
    .progress_chars("█▇▆▅▄▃▂▁  ")
}

struct DownloadStatus {
    bar: ProgressBar,
    downloaded_bytes: AtomicU64,
    downloaded: AtomicU64,
    skipped: AtomicU64,
    redownloading: AtomicU64,
    errors: AtomicU64,
}

impl DownloadStatus {
    fn new(bar: ProgressBar) -> Self {
        let b = DownloadStatus {
            bar,
            downloaded_bytes: Default::default(),
            downloaded: Default::default(),
            skipped: Default::default(),
            redownloading: Default::default(),
            errors: Default::default(),
        };
        b._update();
        b
    }
    fn _update(&self) {
        let m = [
            format!(
                "total bytes: {}",
                DecimalBytes(self.downloaded_bytes.load(Ordering::Acquire))
            ),
            format!("skipped: {}", self.skipped.load(Ordering::Acquire)),
            format!(
                "redownloaded: {}",
                self.redownloading.load(Ordering::Acquire)
            ),
            format!("errors: {}", self.errors.load(Ordering::Acquire)),
        ]
        .join(", ");
        self.bar.set_message(m);
        self.bar.tick();
    }
    fn update(&self, outcome: Option<&Outcome>) {
        let d = self.downloaded.fetch_add(1, Ordering::AcqRel) + 1;
        let (bytes, skip, redown, err) = if let Some(o) = outcome {
            match o {
                Outcome::Download(b) => {
                    let n = self.downloaded_bytes.fetch_add(*b, Ordering::AcqRel) + *b;
                    (Some(n), None, None, None)
                }
                Outcome::Existing => {
                    let s = self.skipped.fetch_add(1, Ordering::AcqRel) + 1;
                    (None, Some(s), None, None)
                }
                Outcome::Redownload(b) => {
                    let n = self.downloaded_bytes.fetch_add(*b, Ordering::AcqRel);
                    let r = self.redownloading.fetch_add(1, Ordering::AcqRel);
                    (Some(n), None, Some(r), None)
                }
            }
        } else {
            let e = self.errors.fetch_add(1, Ordering::AcqRel) + 1;
            (None, None, None, Some(e))
        };
        self._update();
    }
}

async fn download_all(client: Arc<Client>, items: &[(Item, String)], format: Format, dest: &Path) {
    let sem = Arc::new(Semaphore::new(4));
    let mut handles = vec![];
    let mult = MultiProgress::new();
    let p = mult.add(
        ProgressBar::new(items.len() as u64)
            .with_style(bar_style())
            .with_message("Downloading collection"),
    );
    //let p = Arc::new(Mutex::new(DownloadStatus::new(p)));
    let stat = mult.add(ProgressBar::new_spinner().with_style(spin_style()));
    let status = Arc::new(DownloadStatus::new(stat.clone()));

    for (it, url) in items {
        let a = sem.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let url = url.to_owned();
        let p = p.clone();
        // let progress = progress.clone();
        let mut mult = mult.clone();
        let path = dest.join(it.to_path());
        let title = it.display();
        let status = status.clone();
        let j = spawn(async move {
            // let itemp = progress.add(spinner());
            // itemp.start(&title);
            let out = match download_item(&client, &url, format, &path, &title, &mut mult, |_| ())
                .await
                .map_err(|e| format!("Error downloading '{title}: {e})"))
            {
                Ok(o) => {
                    // itemp.stop(format!("{}: downloaded", title));
                    Some(o)
                }
                Err(e) => {
                    mult.suspend(|| eprintln!("{}", e));
                    None
                }
            };
            //p.lock().await.update(out.as_ref());
            status.update(out.as_ref());
            p.inc(1);
            //itemp.stop("finished");
            sleep(Duration::from_secs(1)).await;
            drop(a);
            out
        });
        handles.push(j);
    }
    for h in handles.drain(..) {
        let _i = h.await.unwrap();
    }
    stat.finish();
}

async fn download_item<F>(
    client: &Client,
    url: &str,
    format: Format,
    target: &Path,
    title: &str,
    mult: &mut MultiProgress,
    outcome_cb: F,
) -> Result<Outcome, Box<dyn Error>>
where
    F: Fn(&Outcome),
{
    let s = mult.add(
        //&main_bar,
        ProgressBar::new_spinner()
            .with_style(spin_style())
            .with_message(title.to_string()),
    );
    let u = get_download_link(client, url, format)
        .await
        .map_err(|e| format!("Attempting to get download link: {e}"))?;
    s.finish();
    mult.remove(&s);
    let s = mult.add(
        ProgressBar::new(0)
            .with_style(dl_style())
            .with_message(title.to_owned()),
    );
    let r = http::download_file(
        client,
        &u,
        target,
        Some(|len, pos| {
            s.set_length(len);
            s.set_position(pos);
        }),
    )
    .await?;
    s.finish();
    if let Outcome::Existing = &r {
        mult.remove(&s);
    }
    outcome_cb(&r);
    Ok(r)
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
    let c = try_spin!(format!("getting cookies from {}", options.browser), cookies::get_cookies(options.browser); |_v, s| { s.set_message("got cookies"); })?;
    let client = http::get_client(Some(Arc::new(c)))?;
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

    download_all(Arc::new(client), &items, options.format, &options.target).await;
    Ok(())
}

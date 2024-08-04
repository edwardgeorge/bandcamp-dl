use std::{error::Error, path::Path, process::ExitCode, sync::Arc};

mod api;
mod cookies;
mod file;
mod http;

use console::Emoji;
use cookies::Browser;
use http::Outcome;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use std::time::Duration;
use tokio::{
    spawn,
    sync::{Mutex, Semaphore},
    time::sleep,
};

use api::*;

const S_SPINNER: Emoji = Emoji("◒◐◓◑◇", "•oO0o");

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(e) = run().await {
        eprintln!("Error: {e}");
        ExitCode::FAILURE
    } else {
        println!("Finished!");
        ExitCode::SUCCESS
    }
}

#[inline]
fn spin_style() -> ProgressStyle {
    ProgressStyle::default_spinner().tick_chars(&S_SPINNER.to_string())
}

#[inline]
fn bar_style() -> ProgressStyle {
    ProgressStyle::with_template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")
        .unwrap()
        .progress_chars("##-")
}

struct DownloadStatus {
    bar: ProgressBar,
    downloaded: u64,
    skipped: u64,
    redownloading: u64,
    errors: u64,
}

impl DownloadStatus {
    fn new(bar: ProgressBar) -> Self {
        let b = DownloadStatus {
            bar,
            downloaded: 0,
            skipped: 0,
            redownloading: 0,
            errors: 0,
        };
        b._update();
        b
    }
    fn _update(&self) {
        let m1 = format!("{} skipped", self.skipped);
        let m2 = format!("{} redownloaded", self.redownloading);
        let m3 = format!("{} error", self.errors);
        let l = self.skipped + self.redownloading + self.errors > 0;
        let mut m = vec![
            if self.skipped > 0 { &m1 } else { "" },
            if self.skipped > 0 && (self.redownloading > 0 || self.errors > 0) {
                ", "
            } else {
                ""
            },
            if self.redownloading > 0 { &m2 } else { "" },
            if self.skipped > 0 && self.redownloading > 0 {
                ", "
            } else {
                ""
            },
            if self.errors > 0 { &m3 } else { "" },
        ]
        .join("");
        if l {
            m = format!(" ({m})");
        }
        self.bar.set_position(self.downloaded);
        self.bar.set_message(format!("downloading collection {m}"));
    }
    fn update(&mut self, outcome: Option<&Outcome>) {
        self.downloaded += 1;
        if let Some(o) = outcome {
            match o {
                Outcome::Download => {}
                Outcome::Existing => {
                    self.skipped += 1;
                }
                Outcome::Redownload => {
                    self.redownloading += 1;
                }
            }
        } else {
            self.errors += 1;
        }
        self._update();
    }
}

async fn download_all(client: Arc<Client>, items: &[(Item, String)], dest: &Path) {
    let sem = Arc::new(Semaphore::new(4));
    let mut handles = vec![];
    let mult = MultiProgress::new();
    let p = mult.add(
        ProgressBar::new(items.len() as u64)
            .with_style(bar_style())
            .with_message("Downloading collection"),
    );
    let p = Arc::new(Mutex::new(DownloadStatus::new(p)));

    for (it, url) in items {
        let a = sem.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let url = url.to_owned();
        let p = p.clone();
        // let progress = progress.clone();
        let mut mult = mult.clone();
        let path = dest.join(it.to_path());
        let title = it.display();
        let j = spawn(async move {
            // let itemp = progress.add(spinner());
            // itemp.start(&title);
            let out = match download_item(&client, &url, &path, &title, &mut mult, |_| ())
                .await
                .map_err(|e| format!("Error downloading '{title}: {e})"))
            {
                Ok(o) => {
                    // itemp.stop(format!("{}: downloaded", title));
                    Some(o)
                }
                Err(e) => {
                    p.lock().await.bar.suspend(|| eprintln!("{}", e));
                    None
                }
            };
            p.lock().await.update(out.as_ref());
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
}

async fn download_item<F>(
    client: &Client,
    url: &str,
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
            .with_message(format!("{title}")),
    );
    let u = get_download_link(client, url, "flac")
        .await
        .map_err(|e| format!("Attempting to get download link: {e}"))?;
    s.finish();
    mult.remove(&s);
    let s = mult.add(
        ProgressBar::new(0)
            .with_style(bar_style())
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

async fn run() -> Result<(), Box<dyn Error>> {
    let c = try_spin!("getting cookies", cookies::get_cookies(Browser::Firefox); |_v, s| { s.set_message("got cookies"); })?;
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
            eprintln!("'{}' has no downloads available.", it.display());
            false
        } else {
            true
        }
    })
    .collect::<Vec<_>>();

    download_all(Arc::new(client), &items, Path::new("dl")).await;
    Ok(())
}

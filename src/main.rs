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
use tokio::{spawn, sync::Semaphore, time::sleep};

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

async fn download_all(client: Arc<Client>, items: &[(Item, String)], dest: &Path) {
    let sem = Arc::new(Semaphore::new(4));
    let mut handles = vec![];
    let mult = MultiProgress::new();
    //let p = progress.add(progress_bar(items.len() as u64));
    let p = mult.add(
        ProgressBar::new(items.len() as u64)
            .with_style(bar_style())
            .with_message("Downloading collection"),
    );

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
            let out = match download_item(&client, &url, &path, &title, &mut mult, &p).await {
                Ok(o) => {
                    // itemp.stop(format!("{}: downloaded", title));
                    Some(o)
                }
                Err(e) => {
                    // itemp.error(format!("Error downloading {}: {e}", title));
                    eprintln!("Error downloading '{title}: {e})");
                    None
                }
            };
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
}

async fn download_item(
    client: &Client,
    url: &str,
    target: &Path,
    title: &str,
    mult: &mut MultiProgress,
    main_bar: &ProgressBar,
) -> Result<Outcome, Box<dyn Error>> {
    let s = mult.add(
        //&main_bar,
        ProgressBar::new_spinner()
            .with_style(spin_style())
            .with_message(format!("{title}")),
    );
    let u = get_download_link(client, url, "flac").await?;
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
    .await?;
    download_all(Arc::new(client), &items, Path::new("dl")).await;
    Ok(())
}

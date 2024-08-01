use std::{error::Error, path::Path, process::ExitCode, sync::Arc};

mod api;
mod cookies;
mod file;
mod http;
use cliclack::{intro, multi_progress, outro, outro_cancel, progress_bar, spinner, MultiProgress};
use cookies::Browser;
use reqwest::Client;
use std::time::Duration;
use tokio::{spawn, sync::Semaphore, time::sleep};

use api::*;

#[tokio::main]
async fn main() -> ExitCode {
    intro("bandcamp-dl").unwrap();
    if let Err(e) = run().await {
        //eprintln!("Error: {e}");
        outro_cancel(format!("Error: {e}")).unwrap();
        ExitCode::FAILURE
    } else {
        outro("Finished!").unwrap();
        ExitCode::SUCCESS
    }
}

async fn download_all(
    client: Arc<Client>,
    items: &[(Item, String)],
    dest: &Path,
    progress: &Arc<MultiProgress>,
) {
    let sem = Arc::new(Semaphore::new(4));
    let mut handles = vec![];
    let p = progress.add(progress_bar(items.len() as u64));

    for (it, url) in items {
        let a = sem.clone().acquire_owned().await.unwrap();
        let client = client.clone();
        let url = url.to_owned();
        let p = p.clone();
        let progress = progress.clone();
        let path = dest.join(it.to_path());
        let title = it.display();
        let j = spawn(async move {
            let itemp = progress.add(spinner());
            itemp.start(&title);
            match download_item(&client, &url, &path).await {
                Ok(_) => itemp.stop(format!("{}: downloaded", title)),
                Err(e) => itemp.error(format!("Error downloading {}: {e}", title)),
            }
            p.inc(1);
            //itemp.stop("finished");
            sleep(Duration::from_secs(1)).await;
            drop(a);
        });
        handles.push(j);
    }
    for h in handles.drain(..) {
        h.await.unwrap();
    }
}

async fn download_item(client: &Client, url: &str, target: &Path) -> Result<(), Box<dyn Error>> {
    let u = get_download_link(client, url, "flac").await?;
    http::download_file(client, &u, target).await?;
    Ok(())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let c = cookies::get_cookies(Browser::Firefox)?;
    let client = http::get_client(Some(Arc::new(c)))?;
    let summary = collection_summary(&client).await?;
    let s = spinner();
    s.start(format!(
        "logged in as {} ({}). retrieving profile...",
        summary.collection_summary.username, summary.collection_summary.fan_id,
    ));
    let profile = user_profile(&client, &summary.collection_summary.url)
        .await
        .unwrap();
    s.stop(format!(
        "logged in as {} ({}).",
        summary.collection_summary.username, summary.collection_summary.fan_id,
    ));
    let s = spinner();
    s.start(format!(
        "collection_count: {}. listing remaining collection...",
        profile.collection_count
    ));
    let items = list_remaining_collection(&client, summary.fan_id, &profile).await?;
    s.stop(format!("collection_count: {}.", profile.collection_count));
    let m = Arc::new(multi_progress("downloading..."));
    download_all(Arc::new(client), &items, Path::new("dl"), &m).await;
    m.stop();
    Ok(())
}

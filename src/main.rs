use std::{error::Error, fs::create_dir_all, io::Write, path::Path, process::ExitCode, sync::Arc};

mod api;
mod cookies;
mod http;
use cliclack::{intro, multi_progress, outro, outro_cancel, progress_bar, spinner, MultiProgress};
use futures_util::StreamExt as _;
use reqwest::Client;
use scraper::{Html, Selector};
use std::time::Duration;
use tokio::{spawn, sync::Semaphore, time::sleep};

use api::*;

#[tokio::main]
async fn main() -> ExitCode {
    intro("bandcamp-dl").unwrap();
    if let Err(e) = run().await {
        //eprintln!("Error: {e}");
        outro_cancel(format!("Error: {e}")).unwrap();
        return ExitCode::FAILURE;
    }
    outro("Finished!").unwrap();
    return ExitCode::SUCCESS;
    let c = cookies::get_cookies().unwrap();
    //let client = http::get_client(Some(Arc::new(c))).unwrap();
    let client = http::get_client(None).unwrap();
    let sel = Selector::parse("div#centerWrapper page-footer").unwrap();
    let summary = collection_summary(&client).await.unwrap();

    let r = client
        .get("https://bandcamp.com")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let doc = Html::parse_document(&r.text().await.unwrap());
    for i in doc.select(&sel) {
        let a = i
            .attr("page-context")
            .expect("Should contain page-context attribute")
            .to_string();
        let c: PageContext = serde_json::from_str(&a)
            .map_err(|e| format!("Should deserialize PageContext from page-context attr: {e}"))
            .unwrap();
        if !c.is_logged_in {
            eprintln!("not logged in!");
            return ExitCode::FAILURE;
        }
        let summary = collection_summary(&client).await.unwrap();
        assert_eq!(summary.fan_id, c.fan_id.unwrap());
        println!(
            "logged in as {} ({})",
            summary.collection_summary.username, summary.collection_summary.fan_id,
        );
        let p = user_profile(&client, &summary.collection_summary.url)
            .await
            .unwrap();
        println!("collection_count: {}", p.collection_count);
        let mut remaining = p.collection_count - p.collection_data.batch_size;
        while remaining > 0 {
            println!("{remaining}");
            let items = collection_items(
                &client,
                &CollectionItemsRequest {
                    fan_id: summary.fan_id,
                    count: std::cmp::min(remaining, 500),
                    older_than_token: &p.collection_data.last_token.clone(),
                },
            )
            .await
            .unwrap();
            remaining -= items.items.len();
            if !items.more_available {
                break;
            }
        }
    }
    println!("Hello, world!");
    ExitCode::SUCCESS
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
        cliclack::log::info(format!("{}", it.to_path().display())).unwrap();
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
    download_file(client, &u, target).await?;
    Ok(())
}

async fn download_file(client: &Client, url: &str, target: &Path) -> Result<(), Box<dyn Error>> {
    let r = client.get(url).send().await?.error_for_status()?;
    let len: u64 = r
        .headers()
        .get("Content-length")
        .ok_or("No content-length header")?
        .to_str()?
        .parse()?;
    let disposition = r
        .headers()
        .get("Content-disposition")
        .ok_or("No content-disposition header")?
        .to_str()?;
    let filename = http::filename_from_disposition(disposition)?;
    let target_file = target.join(filename);
    if !target.exists() {
        create_dir_all(target)?;
    } else if target_file.exists() {
        if target_file.is_file() {
            let meta = target_file.metadata()?;
            if meta.len() != len {
                log::info!(
                    "File '{}' is not the expected size... overwriting...",
                    target_file.display()
                );
            } else {
                return Ok(());
            }
        } else {
            return Err(format!(
                "File '{}' already exists and is not a regular file!",
                target_file.display()
            )
            .into());
        }
    }
    let mut f = atomic_write_file::AtomicWriteFile::open(target_file)?;
    let mut bytestream = r.bytes_stream();
    while let Some(v) = bytestream.next().await {
        let b = v?;
        f.write_all(&b)?;
    }
    f.commit()?;
    Ok(())
}

async fn run() -> Result<(), Box<dyn Error>> {
    let c = cookies::get_cookies()?;
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

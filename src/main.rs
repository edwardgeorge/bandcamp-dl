use std::{
    collections::HashMap,
    error::Error,
    fs::create_dir_all,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

mod cookies;
mod http;
use cliclack::{intro, multi_progress, outro, outro_cancel, progress_bar, spinner, MultiProgress};
use futures_util::StreamExt as _;
use lazy_static::lazy_static;
use mailparse::DispositionType;
use percent_encoding::percent_decode_str;
use reqwest::{Client, Response};
use scraper::{Html, Selector};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::{spawn, sync::Semaphore, time::sleep};

lazy_static! {
    //static ref PAGE_FOOTER: Selector = Selector::parse("div#centerWrapper page-footer").unwrap();
    static ref PAGE_DATA: Selector = Selector::parse("div#pagedata").unwrap();
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PageContext {
    #[serde(rename = "fanId")]
    fan_id: Option<u64>,
    #[serde(rename = "userId")]
    user_id: Option<u64>,
    #[serde(rename = "isLoggedIn")]
    is_logged_in: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionSummary {
    fan_id: u64,
    username: String,
    url: String,
    tralbum_lookup: Option<HashMap<String, LookupItem>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CollectionSummaryResult {
    fan_id: u64,
    collection_summary: CollectionSummary,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CollectionItemsRequest<'a> {
    fan_id: u64,
    count: usize,
    older_than_token: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LookupItem {
    item_type: String,
    item_id: u64,
    band_id: u64,
    purchased: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProfileData {
    collection_count: usize,
    item_cache: ItemCache,
    collection_data: CollectionData,
    #[serde(flatten)]
    other: Value,
}

impl ProfileData {
    pub fn iter_collection<'a>(&'a self) -> impl Iterator<Item = (Item, String)> + 'a {
        self.collection_data.sequence.iter().map(|id| {
            let it = self.item_cache.collection.get(id).unwrap();
            let u = self
                .collection_data
                .redownload_urls
                .get(&it.download_key().unwrap())
                .unwrap();
            (it.clone(), u.to_string())
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadData {
    download_items: Vec<DownloadItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DownloadItem {
    downloads: HashMap<String, Download>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Download {
    description: String,
    encoding_name: String,
    size_mb: String,
    url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CollectionData {
    batch_size: usize,
    hidden_items_count: usize,
    last_token: String,
    item_count: usize,
    redownload_urls: HashMap<String, String>,
    sequence: Vec<String>,
    #[serde(flatten)]
    other: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ItemCache {
    collection: HashMap<String, Item>,
    hidden: HashMap<String, Item>,
    wishlist: HashMap<String, Item>,
    #[serde(flatten)]
    other: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CollectionItemsResult {
    more_available: bool,
    items: Vec<Item>,
    redownload_urls: HashMap<String, String>,
    last_token: String,
}

impl CollectionItemsResult {
    pub fn iter_collection<'a>(&'a self) -> impl Iterator<Item = (Item, String)> + 'a {
        self.items.iter().map(|it| {
            let u = self
                .redownload_urls
                .get(&it.download_key().unwrap())
                .unwrap();
            (it.clone(), u.to_owned())
        })
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Item {
    album_id: Option<u64>,
    also_collected_count: usize,
    band_id: u64,
    band_name: String,
    download_available: Option<bool>,
    hidden: Option<Value>,
    is_private: bool,
    is_preorder: bool,
    is_purchasable: bool,
    is_subscriber_only: bool,
    is_subscription_item: bool,
    item_id: u64,
    item_title: String,
    item_type: String,
    item_url: String,
    purchased: Option<String>,
    tralbum_id: u64,
    tralbum_type: String,
    sale_item_id: Option<u64>,
    sale_item_type: Option<String>,
    #[serde(flatten)]
    other: Value,
}

impl Item {
    pub fn display(&self) -> String {
        format!("{} - {}", self.item_title, self.band_name)
    }
    pub fn download_key(&self) -> Option<String> {
        Some(format!(
            "{}{}",
            self.sale_item_type.as_ref()?,
            self.sale_item_id.as_ref()?
        ))
    }
    pub fn to_path(&self) -> PathBuf {
        let level1 = sanitize_filename::sanitize(&self.band_name);
        let level2 = format!(
            "{level1} - {}",
            sanitize_filename::sanitize(&self.item_title),
        );
        Path::new(&level1).join(&level2)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaybeError {
    #[serde(default)]
    error: bool,
    error_message: Option<String>,
    #[serde(flatten)]
    resp: Value,
}

async fn resp_deser<T>(resp: Response) -> Result<T, Box<dyn Error>>
where
    T: DeserializeOwned,
{
    let t = resp.text().await?;
    let v: MaybeError = serde_json::from_str(&t)?;
    if v.error {
        return Err(v
            .error_message
            .unwrap_or_else(|| "unknown error from api".to_owned())
            .into());
    }
    Ok(serde_json::from_value(v.resp).map_err(|e| format!("Error deserialising response: {e}"))?)
}

async fn collection_summary(client: &Client) -> Result<CollectionSummaryResult, Box<dyn Error>> {
    let r = client
        .get("https://bandcamp.com/api/fan/2/collection_summary")
        .send()
        .await?
        .error_for_status()?;
    resp_deser(r).await
}

async fn user_profile(client: &Client, url: &str) -> Result<ProfileData, Box<dyn Error>> {
    let r = client.get(url).send().await?.error_for_status()?;
    let doc = Html::parse_document(&r.text().await?);
    for el in doc.select(&PAGE_DATA) {
        let a = el.attr("data-blob").unwrap();
        let v: ProfileData = serde_json::from_str(&a)?;
        return Ok(v);
    }
    Err("No data-blob found in user profile".into())
}

async fn collection_items(
    client: &Client,
    req: &CollectionItemsRequest<'_>,
) -> Result<CollectionItemsResult, Box<dyn Error>> {
    let r = client
        .post("https://bandcamp.com/api/fancollection/1/collection_items")
        .json(req)
        .header("Content-type", "application/json")
        .header("X-Requested-With", "XMLHttpRequest")
        .send()
        .await?
        .error_for_status()?;
    resp_deser(r).await
}

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

async fn list_remaining_collection(
    client: &Client,
    fan_id: u64,
    profile: &ProfileData,
) -> Result<Vec<(Item, String)>, Box<dyn Error>> {
    let mut result: Vec<_> = profile.iter_collection().collect();
    let mut remaining = profile.collection_count - profile.collection_data.batch_size;
    let mut last_token = profile.collection_data.last_token.clone();
    while remaining > 0 {
        let items = collection_items(
            &client,
            &CollectionItemsRequest {
                fan_id,
                count: std::cmp::min(remaining, 500),
                older_than_token: &last_token,
            },
        )
        .await
        .unwrap();
        result.extend(items.iter_collection());
        remaining -= items.items.len();
        if !items.more_available {
            break;
        }
        last_token = items.last_token;
    }
    Ok(result)
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

pub fn filename_from_disposition(cd: &str) -> Result<String, Box<dyn Error>> {
    let x = mailparse::parse_content_disposition(cd);
    if let DispositionType::Attachment = x.disposition {
        Ok(x.params
            .get("filename*")
            .and_then(|i| i.strip_prefix("UTF-8''"))
            .and_then(|i| percent_decode_str(i).decode_utf8().ok())
            .or_else(|| {
                x.params
                    .get("filename")
                    .and_then(|i| percent_decode_str(i).decode_utf8().ok())
            })
            .ok_or_else(|| {
                format!("Could not parse a filename from the content-disposition header '{cd}'")
            })?
            .to_string())
    } else {
        Err(format!(
            "Content-disposition is expected to be an attachment with filename param. got '{cd}'"
        )
        .into())
    }
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
    let filename = filename_from_disposition(disposition)?;
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

async fn get_download_link(
    client: &Client,
    url: &str,
    format: &str,
) -> Result<String, Box<dyn Error>> {
    let r = client.get(url).send().await?.error_for_status()?;
    let doc = Html::parse_document(&r.text().await?);
    for el in doc.select(&PAGE_DATA) {
        let a = el
            .attr("data-blob")
            .ok_or("data-blob attribute not found")?;
        let v: DownloadData = serde_json::from_str(&a)?;
        let d = v
            .download_items
            .first()
            .ok_or("No download items present!")?;
        let u = d.downloads.get(format).ok_or_else(|| {
            format!(
                "format {format} not available in: {}",
                d.downloads.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        return Ok(u.url.to_owned());
    }
    Err(format!("No page-data for download at: {}", url).into())
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

use std::{error::Error, io::Write, path::Path, sync::Arc};

use futures_util::StreamExt as _;
use mailparse::DispositionType;
use percent_encoding::percent_decode_str;
use reqwest::Client;
use reqwest_cookie_store::CookieStoreMutex;
use tokio::fs::create_dir_all;

pub fn get_client(cs: Option<Arc<CookieStoreMutex>>) -> Result<Client, Box<dyn Error>> {
    let mut cb = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:122.0) Gecko/20100101 Firefox/122.0",
        )
        .cookie_store(true)
        .gzip(true);
    cb = match cs {
        Some(v) => cb.cookie_provider(v),
        None => cb.cookie_store(true),
    };
    Ok(cb.build()?)
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

pub async fn download_file(
    client: &Client,
    url: &str,
    target: &Path,
) -> Result<(), Box<dyn Error>> {
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
    let filename = crate::http::filename_from_disposition(disposition)?;
    let target_file = target.join(filename);
    if !target.exists() {
        create_dir_all(target).await?;
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
        // TODO: this needs to be async
        f.write_all(&b)?;
    }
    f.commit()?;
    Ok(())
}

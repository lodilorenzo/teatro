use std::io;

use axum::{
    body::Body,
    http::{
        HeaderValue, Response, StatusCode,
        header::{CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE},
    },
};
use thiserror::Error;
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

use crate::storage::paths::safe_content_disposition_filename;

#[derive(Debug, Error)]
pub enum StreamFileError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error("invalid response header: {0}")]
    InvalidHeader(#[from] axum::http::header::InvalidHeaderValue),

    #[error("failed to build response: {0}")]
    BuildResponse(#[from] axum::http::Error),
}

pub async fn stream_file(
    file: tokio::fs::File,
    content_type: &'static str,
    download_file_name: Option<&str>,
) -> Result<Response<Body>, StreamFileError> {
    let content_length = file.metadata().await?.len();
    stream_reader(file, content_length, content_type, download_file_name)
}

pub fn stream_reader<R>(
    reader: R,
    content_length: u64,
    content_type: &'static str,
    download_file_name: Option<&str>,
) -> Result<Response<Body>, StreamFileError>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let stream = ReaderStream::new(reader);
    let body = Body::from_stream(stream);

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, content_length.to_string());

    if let Some(file_name) = download_file_name {
        let file_name = safe_content_disposition_filename(file_name);
        let content_disposition = format!("attachment; filename=\"{file_name}\"");
        builder = builder.header(
            CONTENT_DISPOSITION,
            HeaderValue::from_str(&content_disposition)?,
        );
    }

    Ok(builder.body(body)?)
}

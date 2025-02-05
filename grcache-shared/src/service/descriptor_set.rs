use std::future::Future;

use base64::{prelude::BASE64_STANDARD, Engine};
use protobuf::{descriptor::FileDescriptorSet, Message};
use sha2::{Digest, Sha256};
use tokio::{fs::File, io::AsyncReadExt};

use crate::config::crd::DescriptorSetSource;

pub struct DummyPanicContext;
impl LoadFromBucket for DummyPanicContext {
    async fn file_from_bucket(
        &self,
        _bucket_id: &str,
        _object_name: &str,
    ) -> Result<Vec<u8>, anyhow::Error> {
        panic!()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error while retrieving file: {0}")]
    FileRetrieval(#[from] std::io::Error),
    #[error("error while decoding base64 data")]
    Base64DecodeError(#[from] base64::DecodeError),
    #[error("error during proto parse: {0}")]
    ProtoParse(#[from] protobuf::Error),
    #[error("error fetching descriptor set from bucket `{bucket_name}`: {error}")]
    BucketLoad {
        error: anyhow::Error,
        bucket_name: String,
    },
    #[error("checksum mismatch when loading descriptor set: {reference} != {file}")]
    ChecksumMismatch { reference: String, file: String },
}

/// Trait which implements loading an object from a configured bucket.
pub trait LoadFromBucket {
    fn file_from_bucket(
        &self,
        bucket_id: &str,
        object_name: &str,
    ) -> impl Future<Output = Result<Vec<u8>, anyhow::Error>> + Sync;
}

/// Loads a `FileDescriptorSet` from a `DescriptorSetSource`.
pub async fn from_source(
    context: &impl LoadFromBucket,
    source: &DescriptorSetSource,
) -> Result<FileDescriptorSet, Error> {
    let buf = match source {
        DescriptorSetSource::File { path } => {
            let mut file = File::open(path).await?;

            let mut buf = Vec::new();
            file.read_to_end(&mut buf).await?;

            buf
        }
        DescriptorSetSource::Bucket { name, hash } => {
            let buf = context
                .file_from_bucket(name, hash)
                .await
                .map_err(|error| Error::BucketLoad {
                    error,
                    bucket_name: name.clone(),
                })?;

            let file_hash = sha256_hex(&buf);

            if hash != &file_hash {
                return Err(Error::ChecksumMismatch {
                    reference: hash.clone(),
                    file: file_hash.clone(),
                });
            }

            buf
        }
        DescriptorSetSource::Inline { data } => BASE64_STANDARD.decode(&data)?,
    };

    let descriptor_set = protobuf::descriptor::FileDescriptorSet::parse_from_bytes(&buf)?;
    Ok(descriptor_set)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let hash = hasher.finalize();
    format!("{:x}", hash)
}

#[cfg(test)]
mod tests {
    use super::{from_source, sha256_hex, LoadFromBucket};
    use crate::config::crd::DescriptorSetSource;

    struct DummyBucketLoader {
        object_name: String,
        data: Vec<u8>,
    }

    impl LoadFromBucket for DummyBucketLoader {
        async fn file_from_bucket(
            &self,
            _bucket_id: &str,
            object_name: &str,
        ) -> Result<Vec<u8>, anyhow::Error> {
            if object_name == self.object_name {
                Ok(self.data.clone())
            } else {
                anyhow::bail!("no object found")
            }
        }
    }

    #[tokio::test]
    async fn test_descriptor_set_from_bucket() {
        let data = b"";
        let hash = sha256_hex(data);

        let loader = DummyBucketLoader {
            object_name: hash.clone(),
            data: data.into(),
        };

        let source = DescriptorSetSource::Bucket {
            name: "some_bucket".into(),
            hash,
        };

        let result = from_source(&loader, &source).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_descriptor_set_from_bucket_with_wrong_hash() {
        let data = b"";
        let hash = sha256_hex(data);

        let loader = DummyBucketLoader {
            object_name: hash.clone(),
            data: b"a".into(),
        };

        let source = DescriptorSetSource::Bucket {
            name: "some_bucket".into(),
            hash,
        };

        let result = from_source(&loader, &source).await;
        assert!(result.is_err());
    }
}

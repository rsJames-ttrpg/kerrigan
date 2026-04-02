use anyhow::Result;

/// A fetched and decompressed artifact with its associated run_id.
pub struct FetchedArtifact {
    pub run_id: String,
    pub data: Vec<u8>,
}

/// Decompress a gzipped byte slice.
pub fn decompress_gz(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let mut decoder = GzDecoder::new(data);
    let mut buf = Vec::new();
    decoder.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Fetch all artifacts of a given type since a timestamp from Overseer.
/// Returns decompressed artifact data paired with run_id.
pub async fn fetch_artifacts(
    client: &nydus::NydusClient,
    artifact_type: &str,
    since: &chrono::DateTime<chrono::Utc>,
) -> Result<Vec<FetchedArtifact>> {
    let since_str = since.to_rfc3339();
    let artifacts = client
        .list_artifacts(None, Some(artifact_type), Some(&since_str))
        .await
        .map_err(|e| anyhow::anyhow!("failed to list artifacts: {e}"))?;

    let mut results = Vec::new();
    for artifact in artifacts {
        let Some(run_id) = artifact.run_id else {
            tracing::debug!(artifact_id = %artifact.id, "skipping artifact without run_id");
            continue;
        };
        let blob = client
            .get_artifact(&artifact.id)
            .await
            .map_err(|e| anyhow::anyhow!("failed to fetch artifact {}: {e}", artifact.id))?;
        let data = decompress_gz(&blob)?;
        results.push(FetchedArtifact { run_id, data });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    #[test]
    fn test_decompress_gz() {
        let original = b"hello evolution chamber";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let result = decompress_gz(&compressed).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_gz_invalid_data() {
        let result = decompress_gz(b"not gzip data");
        assert!(result.is_err());
    }
}

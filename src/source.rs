use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component as PathComponent, Path, PathBuf};
use tar::Archive;
use tempfile::TempDir;
use url::Url;
use zip::ZipArchive;

const MAX_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 500 * 1024 * 1024;
const MAX_ENTRIES: usize = 20_000;

pub struct PreparedSource {
    pub root: PathBuf,
    pub display: String,
    pub revision: Option<String>,
    _temp: Option<TempDir>,
}

impl PreparedSource {
    fn temporary(root: PathBuf, display: String, revision: Option<String>, temp: TempDir) -> Self {
        Self {
            root,
            display,
            revision,
            _temp: Some(temp),
        }
    }

    fn local(root: PathBuf, display: String) -> Self {
        Self {
            root,
            display,
            revision: None,
            _temp: None,
        }
    }
}

pub trait SourceProvider {
    fn prepare(&self, source: &str) -> Result<PreparedSource>;
}

pub struct DefaultSourceProvider {
    client: Client,
}

impl Default for DefaultSourceProvider {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .user_agent(concat!("agentport/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("valid HTTP client"),
        }
    }
}

impl SourceProvider for DefaultSourceProvider {
    fn prepare(&self, source: &str) -> Result<PreparedSource> {
        if source.starts_with("https://github.com/") || source.starts_with("http://github.com/") {
            return self.prepare_github(source);
        }

        let path = PathBuf::from(source);
        if path.is_dir() {
            return Ok(PreparedSource::local(
                path.canonicalize().context("resolve local source")?,
                source.to_owned(),
            ));
        }
        if !path.is_file() {
            bail!("source is neither a supported GitHub URL nor a local file/directory");
        }

        let temp = tempfile::tempdir().context("create extraction directory")?;
        let lower = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if lower.ends_with(".zip") {
            extract_zip(&path, temp.path())?;
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            extract_tar_gz(&path, temp.path())?;
        } else {
            bail!("unsupported archive; expected .zip, .tar.gz, or .tgz");
        }
        let root = single_root(temp.path())?;
        Ok(PreparedSource::temporary(
            root,
            source.to_owned(),
            None,
            temp,
        ))
    }
}

impl DefaultSourceProvider {
    fn prepare_github(&self, source: &str) -> Result<PreparedSource> {
        let parsed = Url::parse(source).context("parse GitHub URL")?;
        let segments: Vec<_> = parsed
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty())
            .collect();
        if segments.len() < 2 {
            bail!("GitHub URL must include owner and repository");
        }
        let owner = segments[0];
        let repo = segments[1].trim_end_matches(".git");
        let (revision, subpath) = if segments.get(2) == Some(&"tree") {
            let revision = segments
                .get(3)
                .context("GitHub tree URL is missing a ref")?
                .to_string();
            let subpath = segments.get(4..).unwrap_or_default().join("/");
            (Some(revision), subpath)
        } else {
            (None, String::new())
        };
        let archive_ref = revision.as_deref().unwrap_or("HEAD");
        let archive_url = format!("https://github.com/{owner}/{repo}/archive/{archive_ref}.zip");
        let temp = tempfile::tempdir().context("create download directory")?;
        let archive_path = temp.path().join("source.zip");
        self.download(&archive_url, &archive_path)?;
        let unpacked = temp.path().join("unpacked");
        fs::create_dir(&unpacked)?;
        extract_zip(&archive_path, &unpacked)?;
        let mut root = single_root(&unpacked)?;
        if !subpath.is_empty() {
            root = root.join(&subpath);
            if !root.is_dir() {
                bail!("path '{subpath}' was not found in the downloaded repository");
            }
        }
        Ok(PreparedSource::temporary(
            root,
            source.to_owned(),
            revision,
            temp,
        ))
    }

    fn download(&self, url: &str, path: &Path) -> Result<()> {
        let mut response = self
            .client
            .get(url)
            .send()
            .context("download GitHub archive")?;
        if !response.status().is_success() {
            bail!("GitHub returned {} for {url}", response.status());
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_DOWNLOAD_BYTES)
        {
            bail!("archive exceeds the 100 MiB download limit");
        }
        let mut output = File::create(path)?;
        let mut limited = response.by_ref().take(MAX_DOWNLOAD_BYTES + 1);
        let copied = std::io::copy(&mut limited, &mut output)?;
        if copied > MAX_DOWNLOAD_BYTES {
            bail!("archive exceeds the 100 MiB download limit");
        }
        output.flush()?;
        Ok(())
    }
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, PathComponent::Normal(_) | PathComponent::CurDir))
}

fn extract_zip(path: &Path, destination: &Path) -> Result<()> {
    let file = File::open(path).context("open ZIP archive")?;
    let mut archive = ZipArchive::new(file).context("read ZIP archive")?;
    if archive.len() > MAX_ENTRIES {
        bail!("archive contains too many entries");
    }
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let enclosed = entry
            .enclosed_name()
            .context("archive contains an unsafe path")?
            .to_owned();
        if !safe_relative(&enclosed) {
            bail!("archive contains an unsafe path");
        }
        let unix_mode = entry.unix_mode().unwrap_or_default();
        if unix_mode & 0o170000 == 0o120000 {
            bail!("archive contains a symbolic link");
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_EXPANDED_BYTES {
            bail!("expanded archive exceeds the 500 MiB limit");
        }
        let output = destination.join(enclosed);
        if entry.is_dir() {
            fs::create_dir_all(&output)?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(&output)?;
            std::io::copy(&mut entry, &mut file)?;
        }
    }
    Ok(())
}

fn extract_tar_gz(path: &Path, destination: &Path) -> Result<()> {
    let file = File::open(path).context("open tar.gz archive")?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut entries = 0_usize;
    let mut expanded = 0_u64;
    for item in archive.entries().context("read tar.gz archive")? {
        let mut entry = item?;
        entries += 1;
        if entries > MAX_ENTRIES {
            bail!("archive contains too many entries");
        }
        let path = entry.path()?.into_owned();
        if !safe_relative(&path) {
            bail!("archive contains an unsafe path");
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            bail!("archive contains a link");
        }
        if !(kind.is_dir() || kind.is_file()) {
            bail!("archive contains an unsupported entry type");
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_EXPANDED_BYTES {
            bail!("expanded archive exceeds the 500 MiB limit");
        }
        let output = destination.join(path);
        if kind.is_dir() {
            fs::create_dir_all(&output)?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            entry.unpack(&output)?;
        }
    }
    Ok(())
}

fn single_root(path: &Path) -> Result<PathBuf> {
    let entries: Vec<_> = fs::read_dir(path)?.filter_map(Result::ok).collect();
    if entries.len() == 1 && entries[0].path().is_dir() {
        Ok(entries[0].path())
    } else {
        Ok(path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn rejects_zip_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let zip_path = temp.path().join("bad.zip");
        let file = File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("/absolute", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"unsafe").unwrap();
        writer.finish().unwrap();
        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        assert!(extract_zip(&zip_path, &out).is_err());
    }

    #[test]
    fn extracts_normal_zip() {
        let temp = tempfile::tempdir().unwrap();
        let zip_path = temp.path().join("ok.zip");
        let file = File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("skill/SKILL.md", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"---\nname: skill\n---\n").unwrap();
        writer.finish().unwrap();
        let out = temp.path().join("out");
        fs::create_dir(&out).unwrap();
        extract_zip(&zip_path, &out).unwrap();
        assert!(out.join("skill/SKILL.md").is_file());
    }
}

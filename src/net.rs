//! HTTPS downloads (checked against SHA-256 sums) and the GitHub API.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{ui, util};

/// The HTTP client. Proxies come from `HTTPS_PROXY`/`ALL_PROXY`, like curl.
pub struct Http {
    agent: ureq::Agent,
    token: Option<String>,
}

/// Where a download's expected checksum comes from.
pub enum Sum<'a> {
    /// A SHA-256 hex digest known in advance (pinned in openqup, or published next to the file).
    Sha256(&'a str),
    /// No checksum is published: rely on HTTPS alone (the result is still printed).
    None,
}

impl Http {
    /// A client with the GitHub token from `OPENQ_GITHUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN` or
    /// the GitHub CLI's login, if any (needed while the OpenQ repository is private).
    pub fn new() -> Self {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier);
        #[cfg(not(target_os = "linux"))]
        let tls = tls.provider(ureq::tls::TlsProvider::NativeTls);
        let config = ureq::Agent::config_builder()
            .tls_config(tls.build())
            .user_agent(format!("openqup/{}", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_body(Some(Duration::from_secs(120)))
            .build();
        let token = ["OPENQ_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"]
            .iter()
            .find_map(|v| std::env::var(v).ok().filter(|t| !t.trim().is_empty()))
            .or_else(|| util::output(Command::new("gh").args(["auth", "token"])).filter(|t| !t.is_empty()));
        Self { agent: config.into(), token }
    }

    /// Whether a GitHub token is available.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn get(&self, url: &str, accept: &str) -> Result<ureq::http::Response<ureq::Body>> {
        let mut req = self.agent.get(url).header("Accept", accept);
        // The token only goes to the GitHub API; redirects elsewhere drop it (ureq's default).
        if url.starts_with("https://api.github.com/") {
            req = req.header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(t) = &self.token {
                req = req.header("Authorization", format!("Bearer {}", t.trim()));
            }
        }
        req.call().map_err(|e| match e {
            ureq::Error::StatusCode(404) if url.starts_with("https://api.github.com/repos/") && self.token.is_none() => {
                anyhow::anyhow!(
                    "{url}: not found (404). If the repository is private, set OPENQ_GITHUB_TOKEN to a GitHub token \
                     that can read it, or log in with `gh auth login`."
                )
            }
            e => anyhow::anyhow!("{url}: {e}"),
        })
    }

    /// A GitHub API call returning JSON.
    pub fn api(&self, path: &str) -> Result<Value> {
        let url = format!("https://api.github.com/{path}");
        let mut resp = self.get(&url, "application/vnd.github+json")?;
        let text = resp.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string()?;
        serde_json::from_str(&text).with_context(|| format!("{url}: bad JSON"))
    }

    /// A GitHub API call returning text (`accept` picks the representation).
    pub fn api_text(&self, path: &str, accept: &str) -> Result<String> {
        let url = format!("https://api.github.com/{path}");
        Ok(self.get(&url, accept)?.body_mut().read_to_string()?.trim().to_string())
    }

    /// A small text file (a checksum list, for example).
    pub fn text(&self, url: &str) -> Result<String> {
        Ok(self.get(url, "*/*")?.body_mut().read_to_string()?)
    }

    /// Downloads `url` to `dest` and checks it. A file already at `dest` with the expected
    /// checksum is kept. Returns the file's SHA-256.
    pub fn download(&self, url: &str, dest: &Path, sum: Sum) -> Result<String> {
        if let Sum::Sha256(want) = sum
            && dest.is_file()
            && sha256_file(dest)?.eq_ignore_ascii_case(want)
        {
            ui::info(format!("{} (already downloaded)", dest.display()));
            return Ok(want.to_ascii_lowercase());
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir)?;
        }
        ui::info(format!("Downloading {url}"));
        let mut resp = self.get(url, "application/octet-stream")?;
        let total = resp.headers().get("content-length").and_then(|v| v.to_str().ok()?.parse().ok());
        let part = dest.with_extension("part");
        let mut file = std::fs::File::create(&part).with_context(|| format!("couldn't create {}", part.display()))?;
        let mut reader = resp.body_mut().with_config().limit(u64::MAX).reader();
        let mut hasher = Sha256::new();
        let mut buf = vec![0; 256 * 1024];
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut progress = ui::Progress::new(&name, total);
        loop {
            let n = reader.read(&mut buf).with_context(|| format!("download of {url} failed"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            progress.add(n as u64);
        }
        progress.finish();
        file.sync_all()?;
        drop(file);
        let got = hex(&hasher.finalize());
        match sum {
            Sum::Sha256(want) if !got.eq_ignore_ascii_case(want) => {
                let _ = std::fs::remove_file(&part);
                bail!("{url}: checksum mismatch (expected {want}, got {got}). The download was deleted.");
            }
            Sum::Sha256(_) => ui::info("Checksum OK (SHA-256)"),
            Sum::None => ui::info(format!("SHA-256 {got} (no published checksum to compare with)")),
        }
        std::fs::rename(&part, dest)?;
        Ok(got)
    }

    #[cfg(windows)]
    /// The files attached to a GitHub repository's releases (newest first),
    /// with GitHub's own SHA-256 digest when it has one.
    pub fn release_assets(&self, repo: &str, per_page: u32) -> Result<Vec<Asset>> {
        let releases = self.api(&format!("repos/{repo}/releases?per_page={per_page}"))?;
        let mut out = Vec::new();
        for r in releases.as_array().into_iter().flatten() {
            let tag = r["tag_name"].as_str().unwrap_or_default().to_string();
            for a in r["assets"].as_array().into_iter().flatten() {
                let digest = a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_string);
                out.push(Asset {
                    tag: tag.clone(),
                    name: a["name"].as_str().unwrap_or_default().to_string(),
                    url: a["browser_download_url"].as_str().unwrap_or_default().to_string(),
                    sha256: digest,
                });
            }
        }
        Ok(out)
    }
}

/// A file attached to a GitHub release.
#[cfg(windows)]
#[derive(Clone)]
pub struct Asset {
    /// The release's tag.
    pub tag: String,
    /// File name.
    pub name: String,
    /// Download URL.
    pub url: String,
    /// SHA-256 (hex), when GitHub has one.
    pub sha256: Option<String>,
}

/// The SHA-256 of a file, in hex.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut f, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The digest in a `.sha256` file (`<hex>` or `<hex>  <name>`).
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let word = text.split_whitespace().next()?;
    (word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit())).then(|| word.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_files() {
        let h = "6f4bef66261261fcb43131be8720bab817d403a09edec7455c371974b90bdb7e";
        assert_eq!(parse_sha256_file(&format!("{h}  *rustup-init.exe\n")).as_deref(), Some(h));
        assert_eq!(parse_sha256_file(&h.to_uppercase()).as_deref(), Some(h));
        assert_eq!(parse_sha256_file("<html>"), None);
        assert_eq!(hex(&Sha256::digest(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}

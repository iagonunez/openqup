//! Getting OpenQ's source: a version from GitHub (as a tarball of one exact commit) or a local
//! folder.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::net::{Http, Sum};
use crate::state::{Dirs, Options};
use crate::{ui, util};

/// A source tree ready to build.
pub struct Source {
    /// The folder with Cargo.toml.
    pub dir: PathBuf,
    /// The exact commit, or "local" for a folder that isn't a git checkout.
    pub commit: String,
    /// What was asked for, for messages ("v0.2.0", "master", a folder).
    pub label: String,
}

/// Picks the reference to build: the one asked for, else the newest `vX.Y.Z` tag, else the
/// default branch.
pub fn pick_ref(http: &Http, opts: &Options) -> Result<String> {
    if let Some(r) = &opts.reference {
        return Ok(r.clone());
    }
    if let Some(url) = &opts.git {
        let tags = git_tags(url)?;
        if let Some(t) = newest_version(tags.iter().map(String::as_str)) {
            return Ok(t.to_string());
        }
        // "ref: refs/heads/master\tHEAD"
        let head = git_remote(&["ls-remote", "--symref", url, "HEAD"])?;
        return head
            .lines()
            .find_map(|l| l.strip_prefix("ref: refs/heads/")?.split_whitespace().next().map(str::to_string))
            .context("the repository has no default branch");
    }
    let tags = list_tags(http, &opts.repo)?;
    if let Some(t) = newest_version(tags.iter().map(String::as_str)) {
        return Ok(t.to_string());
    }
    let repo = http.api(&format!("repos/{}", opts.repo))?;
    repo["default_branch"].as_str().map(str::to_string).context("the repository has no default branch")
}

/// The repository's tags (first 100 from GitHub).
pub fn tags(http: &Http, opts: &Options) -> Result<Vec<String>> {
    match &opts.git {
        Some(url) => git_tags(url),
        None => list_tags(http, &opts.repo),
    }
}

fn list_tags(http: &Http, repo: &str) -> Result<Vec<String>> {
    let tags = http.api(&format!("repos/{repo}/tags?per_page=100"))?;
    Ok(tags.as_array().into_iter().flatten().filter_map(|t| t["name"].as_str().map(str::to_string)).collect())
}

/// The commit a reference points at.
pub fn resolve(http: &Http, opts: &Options, reference: &str) -> Result<String> {
    let Some(url) = &opts.git else { return resolve_github(http, &opts.repo, reference) };
    if is_sha(reference) {
        return Ok(reference.to_ascii_lowercase());
    }
    // A peeled annotated tag (`v1^{}`) is the commit; a branch or lightweight tag is listed as is.
    let out = git_remote(&["ls-remote", url, reference, &format!("{reference}^{{}}")])?;
    let mut lines: Vec<(&str, &str)> = out.lines().filter_map(|l| l.split_once('\t')).collect();
    lines.sort_by_key(|(_, name)| !name.ends_with("^{}"));
    lines.first().map(|(sha, _)| sha.to_string()).with_context(|| format!("no tag or branch `{reference}` in {url}"))
}

fn resolve_github(http: &Http, repo: &str, reference: &str) -> Result<String> {
    let sha = http
        .api_text(&format!("repos/{repo}/commits/{reference}"), "application/vnd.github.sha")
        .with_context(|| format!("no tag, branch or commit `{reference}` in {repo}"))?;
    if !is_sha(&sha) {
        bail!("unexpected answer for `{reference}`: {sha}");
    }
    Ok(sha)
}

/// The commit of a local folder, when it is a git checkout (with "-dirty" for local changes).
pub fn local_commit(dir: &std::path::Path) -> String {
    let git = |args: &[&str]| util::output(Command::new("git").arg("-C").arg(dir).args(args));
    match git(&["rev-parse", "HEAD"]) {
        Some(sha) if git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty()) => format!("{sha}-dirty"),
        Some(sha) => sha,
        None => "local".into(),
    }
}

/// Fetches (or reuses) the source for `commit`.
pub fn fetch(http: &Http, dirs: &Dirs, opts: &Options, label: &str, commit: &str) -> Result<Source> {
    let dir = dirs.src().join(commit);
    if dir.join("Cargo.toml").is_file() && dir.join(".oqman-complete").is_file() {
        ui::info(format!("Source {label} ({}) already unpacked", &commit[..12]));
    } else if let Some(url) = &opts.git {
        util::remove(&dir)?;
        std::fs::create_dir_all(&dir)?;
        let git = |args: &[&str]| util::run(Command::new("git").arg("-C").arg(&dir).args(args));
        git(&["init", "-q"])?;
        git(&["fetch", "-q", "--depth", "1", url, commit])?;
        git(&["checkout", "-q", "FETCH_HEAD"])?;
        util::remove(&dir.join(".git"))?;
        std::fs::write(dir.join(".oqman-complete"), commit)?;
    } else {
        let repo = &opts.repo;
        let archive = dirs.downloads().join(format!("openq-{commit}.tar.gz"));
        // GitHub doesn't publish checksums for source tarballs; the commit id pins the content
        // and the download is over HTTPS.
        http.download(&format!("https://api.github.com/repos/{repo}/tarball/{commit}"), &archive, Sum::None)?;
        util::extract(&archive, &dir, 1)?;
        std::fs::write(dir.join(".oqman-complete"), commit)?;
        let _ = std::fs::remove_file(&archive);
    }
    // Older source trees aren't needed any more (the build cache lives elsewhere).
    for entry in std::fs::read_dir(dirs.src())?.flatten() {
        if entry.path() != dir {
            let _ = util::remove(&entry.path());
        }
    }
    Ok(Source { dir, commit: commit.to_string(), label: label.to_string() })
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn git_remote(args: &[&str]) -> Result<String> {
    let out = Command::new("git").args(args).output().context("git isn't installed")?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn git_tags(url: &str) -> Result<Vec<String>> {
    let out = git_remote(&["ls-remote", "--tags", "--refs", url])?;
    Ok(out.lines().filter_map(|l| l.split_once("refs/tags/").map(|(_, t)| t.trim().to_string())).collect())
}

/// The newest `vX.Y.Z` tag (pre-releases like `v1.0.0-rc1` are skipped).
pub fn newest_version<'a>(tags: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    tags.filter_map(|t| Some((parse_version(t)?, t))).max_by_key(|(v, _)| *v).map(|(_, t)| t)
}

fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v')?.split('.');
    let v = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    parts.next().is_none().then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_tag() {
        let tags = ["v0.9.0", "v0.10.0", "v1.0.0-rc1", "nightly", "v0.10.0.1", "v0.2.11"];
        assert_eq!(newest_version(tags.into_iter()), Some("v0.10.0"));
        assert_eq!(newest_version(["latest"].into_iter()), None);
    }
}

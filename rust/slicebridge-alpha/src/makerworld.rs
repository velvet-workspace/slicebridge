//! Source-locked, read-only MakerWorld provenance and deep-link preview.
//! Port of native/makerworld/origin.go, not a downloader or protocol handler.
//! Never logs signed model URLs or changes browser/OS associations.
use serde_json::{json, Value};
use std::fs;
#[cfg(windows)]
use std::fs::File;
#[cfg(any(windows, target_os = "macos"))]
use std::io::Read;
use std::path::Path;
use url::{Host, Url};

const MAX_LINK: usize = 32_768;
const MAX_METADATA: u64 = 32_769;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub kind: &'static str,
    pub confidence: &'static str,
    pub origin_url: Option<String>,
    pub evidence: Option<&'static str>,
}

impl Source {
    pub fn unknown() -> Self {
        Self {
            kind: "unknown",
            confidence: "unknown",
            origin_url: None,
            evidence: None,
        }
    }
    fn recognized(
        kind: &'static str,
        confidence: &'static str,
        origin: Url,
        evidence: &'static str,
    ) -> Self {
        let mut origin = origin;
        origin.set_query(None);
        origin.set_fragment(None);
        Self {
            kind,
            confidence,
            origin_url: Some(origin.into()),
            evidence: Some(evidence),
        }
    }
    pub fn as_value(&self) -> Value {
        let mut value = json!({"kind": self.kind, "confidence": self.confidence});
        if let Some(origin) = &self.origin_url {
            value["originUrl"] = json!(origin);
        }
        if let Some(evidence) = self.evidence {
            value["evidence"] = json!(evidence);
        }
        value
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub scheme: &'static str,
    pub remote_url: String,
    pub source: Source,
}
impl Link {
    pub fn as_value(&self) -> Value {
        json!({"scheme":self.scheme, "remoteUrl":self.remote_url, "source":self.source.as_value()})
    }
}

// Strict domain comparison, including subdomains; makerworld.com.evil.test is
// not a subdomain of makerworld.com. IP literal and userinfo are forbidden.
fn trusted_url(raw: &str, require_https: bool) -> Option<Url> {
    let input = raw.trim().trim_matches(&['"', '\'', '<', '>'][..]);
    let parsed = Url::parse(input).ok()?;
    if parsed.scheme() != "https" && (require_https || parsed.scheme() != "http") {
        return None;
    }
    if parsed.host().is_none() || !matches!(parsed.host(), Some(Host::Domain(_))) {
        return None;
    }
    // Reject even empty userinfo ("https://@makerworld.com").
    let authority = input
        .split_once("://")?
        .1
        .split(&['/', '?', '#'][..])
        .next()?;
    if authority.contains('@') || parsed.port().is_some_and(|p| p != 443) {
        return None;
    }
    Some(parsed)
}

fn domain_is(host: &str, root: &str) -> bool {
    let host = host.trim_end_matches('.');
    host == root || host.ends_with(&format!(".{root}"))
}

fn classify(raw: &str, evidence: &'static str) -> Option<Source> {
    let url = trusted_url(raw, false)?;
    let domain = url.domain()?;
    if domain_is(domain, "makerworld.com") {
        return Some(Source::recognized("makerworld", "high", url, evidence));
    }
    if domain_is(domain, "bblmw.com") {
        return Some(Source::recognized("bambu_cdn", "medium", url, evidence));
    }
    None
}

/// Native Go Identify(referrer, host): high-confidence MakerWorld referrer
/// overrides a CDN download host; a CDN host alone is only medium-confidence.
pub fn identify(referrer: &str, host: &str) -> Source {
    if let Some(source) = classify(referrer, "download_referrer") {
        if source.kind == "makerworld" {
            return source;
        }
    }
    if let Some(source) = classify(host, "download_host") {
        if source.kind == "makerworld" {
            return source;
        }
    }
    if let Some(source) = classify(host, "bambu_download_host") {
        if source.kind == "bambu_cdn" {
            return source;
        }
    }
    Source::unknown()
}

/// Windows Zone.Identifier ADS. The 32 KiB cap matches Go even for malformed
/// or malicious ADS; caller controls no destination path for writes.
pub fn from_windows_ads(data: &[u8]) -> Source {
    if data.len() > 32 * 1024 {
        return Source::unknown();
    }
    let raw = String::from_utf8_lossy(data);
    let (mut referrer, mut host) = ("", "");
    for line in raw.lines() {
        if let Some((key, value)) = line.trim().split_once('=') {
            match key.trim().to_ascii_lowercase().as_str() {
                "referrerurl" => referrer = value,
                "hosturl" => host = value,
                _ => {}
            }
        }
    }
    identify(referrer, host)
}

/// macOS mdls kMDItemWhereFroms: inspect URL strings, prefer MakerWorld over
/// a CDN when both appear, and never include signed query params in receipts.
pub fn from_mac_where_from(raw: &str) -> Source {
    let mut first_cdn = None;
    for candidate in raw
        .split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '(' | ')' | ','))
    {
        let candidate = candidate.trim_end_matches(&['.', ';'][..]);
        let Some(source) = classify(candidate, "mac_where_from") else {
            continue;
        };
        if source.kind == "makerworld" {
            return source;
        }
        if source.kind == "bambu_cdn" && first_cdn.is_none() {
            first_cdn = Some(source);
        }
    }
    first_cdn.unwrap_or_else(Source::unknown)
}

/// Read existing download provenance. Never scans directories, alters source,
/// registers a deep link, makes a network request, or runs a slicer.
pub fn detect(path: &Path) -> Source {
    let none = Source::unknown();
    if !path
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("3mf"))
        || !fs::metadata(path).is_ok_and(|m| m.is_file())
    {
        return none;
    }
    #[cfg(windows)]
    {
        let ads = format!("{}:Zone.Identifier", path.display());
        let Ok(file) = File::open(ads) else {
            return none;
        };
        let mut data = Vec::new();
        if file.take(MAX_METADATA).read_to_end(&mut data).is_err() {
            return none;
        }
        return from_windows_ads(&data);
    }
    #[cfg(target_os = "macos")]
    {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let Ok(mut child) = Command::new("/usr/bin/mdls")
            .arg("-raw")
            .arg("-name")
            .arg("kMDItemWhereFroms")
            .arg(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return none;
        };
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let Some(stdout) = child.stdout.take() else {
                        return none;
                    };
                    let mut limited = Vec::new();
                    if !status.success()
                        || stdout.take(MAX_METADATA).read_to_end(&mut limited).is_err()
                        || limited.len() > 32 * 1024
                    {
                        return none;
                    }
                    return from_mac_where_from(&String::from_utf8_lossy(&limited));
                }
                Ok(None) if started.elapsed() < Duration::from_millis(1500) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return none;
                }
            }
        }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        none
    }
}

fn unwrap_url(mut encoded: String) -> Result<String, String> {
    if encoded.trim().is_empty() || encoded.len() > MAX_LINK {
        return Err("invalid external link length".into());
    }
    encoded = encoded.trim().to_owned();
    for _ in 0..2 {
        if !encoded.to_ascii_lowercase().starts_with("https%3a%2f%2f") {
            break;
        }
        encoded = percent_encoding::percent_decode_str(&encoded)
            .decode_utf8()
            .map_err(|_| "invalid encoded link")?
            .replace('+', " ");
    }
    Ok(encoded)
}

/// Strict preview matching Go ParseLink. It returns the complete signed URL to
/// the direct caller, but does not fetch it or include it in an error message.
pub fn parse_link(raw: &str) -> Result<Link, String> {
    if raw.len() > MAX_LINK {
        return Err("external link too long".into());
    }
    let lower = raw.to_ascii_lowercase();
    let (scheme, encoded) = if lower.starts_with("bambustudio://") {
        let outer = Url::parse(raw).map_err(|_| "unsupported Bambu Studio deep link")?;
        if outer.host_str() != Some("open") {
            return Err("unsupported Bambu Studio deep link".into());
        }
        let value = outer
            .query_pairs()
            .find(|(key, _)| key == "file")
            .map(|(_, val)| val.into_owned())
            .unwrap_or_default();
        ("bambustudio", value)
    } else if lower.starts_with("bambustudioopen://") {
        (
            "bambustudioopen",
            raw["bambustudioopen://".len()..].to_owned(),
        )
    } else if lower.starts_with("slicebridge://") {
        let outer = Url::parse(raw).map_err(|_| "unsupported SliceBridge deep link")?;
        if outer.host_str() != Some("import") {
            return Err("unsupported SliceBridge deep link".into());
        }
        let value = outer
            .query_pairs()
            .find(|(key, _)| key == "url")
            .map(|(_, val)| val.into_owned())
            .unwrap_or_default();
        ("slicebridge", value)
    } else {
        return Err("unsupported external link scheme".into());
    };
    let decoded = unwrap_url(encoded)?;
    let url = trusted_url(&decoded, true).ok_or("only HTTPS model sources are accepted")?;
    let source = classify(&decoded, "download_host")
        .filter(|s| s.kind == "makerworld" || s.kind == "bambu_cdn")
        .ok_or("external URL is not from MakerWorld or a known Bambu CDN")?;
    let source = Source {
        evidence: Some(if source.kind == "makerworld" {
            "download_referrer"
        } else {
            "bambu_download_host"
        }),
        ..source
    };
    // Go Identify(decoded, decoded) uses download_referrer for MakerWorld.
    Ok(Link {
        scheme,
        remote_url: url.as_str().to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn makerworld_source_precedence_and_no_signed_query_leaks() {
        let got = identify(
            "https://makerworld.com/en/models/12345",
            "https://public-cdn.bblmw.com/a.3mf?sig=PRIVATE",
        );
        assert_eq!(got.kind, "makerworld");
        assert_eq!(got.confidence, "high");
        assert_eq!(got.evidence, Some("download_referrer"));
        assert_eq!(
            got.origin_url.as_deref(),
            Some("https://makerworld.com/en/models/12345")
        );
        let cdn = identify("", "https://public-cdn.bblmw.com/example.3mf?token=SECRET");
        assert_eq!(cdn.kind, "bambu_cdn");
        assert_eq!(cdn.evidence, Some("bambu_download_host"));
        assert_eq!(
            cdn.origin_url.as_deref(),
            Some("https://public-cdn.bblmw.com/example.3mf")
        );
        assert_eq!(
            identify("", "https://cdn.example.org/a?destination=makerworld.com"),
            Source::unknown()
        );
        assert_eq!(
            identify("https://makerworld.com.evil.test/a", ""),
            Source::unknown()
        );
        assert_eq!(
            identify("https://makerworld.com@evil.test/a", ""),
            Source::unknown()
        );
        assert_eq!(identify("http://127.0.0.1/a", ""), Source::unknown());
        assert_eq!(
            identify("https://makerworld.com:8443/a", ""),
            Source::unknown()
        );
        assert_eq!(identify("https://@makerworld.com/a", ""), Source::unknown());
        assert_eq!(
            identify("https://en.makerworld.com/a", "").kind,
            "makerworld"
        );
    }

    #[test]
    fn metadata_parsers_match_go_with_caps_and_no_false_identification() {
        let raw = b"[ZoneTransfer]\r\nHostUrl=https://public-cdn.bblmw.com/file.3mf?sig=SECRET\r\nReferrerUrl=https://makerworld.com/en/models/123-example\r\n";
        let source = from_windows_ads(raw);
        assert_eq!(source.kind, "makerworld");
        assert_eq!(source.evidence, Some("download_referrer"));
        assert_eq!(
            source.origin_url.as_deref(),
            Some("https://makerworld.com/en/models/123-example")
        );
        assert_eq!(from_windows_ads(&vec![b'x'; 33_000]), Source::unknown());
        let spotted = from_mac_where_from(
            "(\"https://public-cdn.bblmw.com/a.3mf?token=SECRET\",\"https://makerworld.com/en/models/12345\")"
        );
        assert_eq!(spotted.kind, "makerworld");
        assert_eq!(spotted.evidence, Some("mac_where_from"));
        assert_eq!(
            spotted.origin_url.as_deref(),
            Some("https://makerworld.com/en/models/12345")
        );
        assert_eq!(from_mac_where_from("(null)"), Source::unknown());
    }

    #[test]
    fn deep_link_preview_is_non_executing_and_preserves_signed_url() {
        let cases = [
            ("bambustudio://open?file=https%3A%2F%2Fmakerworld.com%2Fapi%2Fdownload%2Ffile.3mf%3Fsig%3DSECRET",
                "bambustudio", "makerworld"),
            ("bambustudioopen://https%3A%2F%2Fpublic-cdn.bblmw.com%2Fmodel%2F123.3mf%3Ftoken%3D1",
                "bambustudioopen", "bambu_cdn"),
            ("slicebridge://import?url=https%3A%2F%2Fmakerworld.com%2Fen%2Fmodels%2F123",
                "slicebridge", "makerworld"),
        ];
        for (raw, scheme, kind) in cases {
            let link = parse_link(raw).unwrap();
            assert_eq!(link.scheme, scheme);
            assert_eq!(link.source.kind, kind);
            assert_eq!(
                link.source.evidence,
                Some(if kind == "makerworld" {
                    "download_referrer"
                } else {
                    "bambu_download_host"
                })
            );
        }
        let signed = parse_link(cases[0].0).unwrap();
        assert!(signed.remote_url.ends_with("sig=SECRET"));
        assert!(!signed.source.origin_url.unwrap().contains("SECRET"));
    }

    #[test]
    fn invalid_protocol_hosts_ports_schemes_and_length_fail_closed() {
        for raw in [
            "",
            "https://makerworld.com/model",
            "bambustudio://open?file=http%3A%2F%2Fmakerworld.com%2Fmodel",
            "bambustudio://open?file=file%3A%2F%2Fetc%2Fpasswd",
            "bambustudio://open?file=https%3A%2F%2Fmakerworld.com.evil.test%2Ffile",
            "bambustudio://other?file=https%3A%2F%2Fmakerworld.com%2Ffile",
            "slicebridge://notimport?url=https%3A%2F%2Fmakerworld.com%2Ffile",
            "bambustudioopen://https%3A%2F%2F127.0.0.1%2Ffile",
            "bambustudioopen://https%3A%2F%2Fmakerworld.com%3A9999%2Ffile",
            "bambustudioopen://https%3A%2F%2Fmakerworld.com%40evil.test%2Ffile",
            "bambustudioopen://https%3A%2F%2Fevil.test%2Ffile",
        ] {
            assert!(parse_link(raw).is_err(), "incorrectly accepted link");
        }
        assert!(parse_link(&"a".repeat(40_000)).is_err());
    }

    #[test]
    fn unknown_or_missing_local_3mf_never_claims_makerworld() {
        let base =
            std::env::temp_dir().join(format!("sb-makerworld-readonly-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let path = base.join("model.3mf");
        fs::write(&path, b"QA metadata fixture").unwrap();
        let original = fs::read(&path).unwrap();
        assert_eq!(detect(&path), Source::unknown());
        assert_eq!(detect(&base.join("absent.3mf")), Source::unknown());
        assert_eq!(detect(&base.join("model.stl")), Source::unknown());
        assert_eq!(fs::read(&path).unwrap(), original);
        fs::remove_dir_all(base).unwrap();
    }
}

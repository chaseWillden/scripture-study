//! Pasted links: recognizing a URL, stripping tracking junk from it, and
//! giving it a short readable label.

/// Query parameters that only track where a click came from.
const TRACKING_PARAMS: &[&str] = &[
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "twclid",
    "ttclid",
    "igshid",
    "igsh",
    "mc_cid",
    "mc_eid",
    "_hsenc",
    "_hsmi",
    "mkt_tok",
    "si",
    "ref_src",
    "ref_url",
    "s_cid",
    "vero_id",
    "oly_anon_id",
    "oly_enc_id",
    "rb_clickid",
    "_ga",
    "_gl",
    "spm",
    "trk",
    "trkCampaign",
];
const TRACKING_PREFIXES: &[&str] = &["utm_", "pf_rd_", "pd_rd_", "__s"];
/// Parameters that are tracking only on X / Twitter (elsewhere `t` can be a
/// video timestamp).
const SOCIAL_PARAMS: &[&str] = &["s", "t", "ref"];
const SOCIAL_HOSTS: &[&str] = &["x.com", "twitter.com", "mobile.twitter.com"];

/// Labels longer than this are shortened around the middle of the path.
const MAX_LABEL: usize = 48;

/// If `text` is a single web address, the URL (with a scheme added to a bare
/// `www.` address).
pub fn as_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let url = if text.starts_with("https://") || text.starts_with("http://") {
        text.to_string()
    } else if text.starts_with("www.") && text.contains('.') {
        format!("https://{text}")
    } else {
        return None;
    };
    host(&url)
        .is_some_and(|h| h.contains('.') || h == "localhost")
        .then_some(url)
}

/// The URL without tracking parameters or an empty `?`/`#`.
pub fn clean(url: &str) -> String {
    let (rest, fragment) = match url.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (url, None),
    };
    let (base, query) = match rest.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (rest, None),
    };
    let social = host(url).is_some_and(|h| SOCIAL_HOSTS.contains(&h));
    let kept: Vec<&str> = query
        .into_iter()
        .flat_map(|q| q.split('&'))
        .filter(|pair| {
            let key = pair.split('=').next().unwrap_or("");
            let tracking = TRACKING_PARAMS.contains(&key)
                || TRACKING_PREFIXES.iter().any(|p| key.starts_with(p))
                || (social && SOCIAL_PARAMS.contains(&key));
            !key.is_empty() && !tracking
        })
        .collect();

    let mut out = base.to_string();
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(fragment) = fragment.filter(|f| !f.is_empty()) {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// A short, readable name for a URL: host and path without the scheme,
/// `www.`, query, or trailing slash.
pub fn label(url: &str) -> String {
    let Some(host) = host(url) else {
        return url.to_string();
    };
    let host = host.strip_prefix("www.").unwrap_or(host);
    let after_host = url
        .split_once("://")
        .map_or("", |(_, rest)| rest.find('/').map_or("", |n| &rest[n..]));
    let path = after_host
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    let path = decode_spaces(path);
    let full = format!("{host}{path}");
    if full.chars().count() <= MAX_LABEL {
        return full;
    }
    // Keep the host and the last path segment, which usually names the page.
    let last = path.rsplit('/').next().unwrap_or("");
    let short = format!("{host}/…/{last}");
    if short.chars().count() <= MAX_LABEL {
        return short;
    }
    let mut out: String = full.chars().take(MAX_LABEL - 1).collect();
    out.push('…');
    out
}

/// A Markdown link for a pasted URL, labeled `text` or a readable label.
pub fn markdown(url: &str, text: Option<&str>) -> String {
    let url = clean(url);
    let text = match text.map(str::trim).filter(|t| !t.is_empty()) {
        Some(text) => text.to_string(),
        None => label(&url),
    };
    format!(
        "[{}]({})",
        text.replace(['[', ']'], ""),
        url.replace(' ', "%20")
    )
}

fn host(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

fn decode_spaces(path: &str) -> String {
    path.replace("%20", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_single_urls_only() {
        assert_eq!(
            as_url(" https://example.com/a \n").as_deref(),
            Some("https://example.com/a")
        );
        assert_eq!(
            as_url("www.rust-lang.org").as_deref(),
            Some("https://www.rust-lang.org")
        );
        assert_eq!(as_url("see https://example.com"), None);
        assert_eq!(as_url("example.com"), None);
        assert_eq!(as_url("https://"), None);
    }

    #[test]
    fn strips_tracking_parameters() {
        assert_eq!(
            clean("https://shop.io/item?id=7&utm_source=x&utm_medium=y&fbclid=abc#"),
            "https://shop.io/item?id=7"
        );
        assert_eq!(
            clean("https://youtu.be/abc?si=XYZ&t=42"),
            "https://youtu.be/abc?t=42",
            "t is a timestamp outside X"
        );
        assert_eq!(
            clean("https://x.com/user/status/1?s=20&t=abc"),
            "https://x.com/user/status/1"
        );
        assert_eq!(
            clean("https://a.io/p?utm_campaign=z#section"),
            "https://a.io/p#section"
        );
    }

    #[test]
    fn labels_are_short_and_readable() {
        assert_eq!(
            label("https://www.github.com/owner/repo/pull/12/"),
            "github.com/owner/repo/pull/12"
        );
        assert_eq!(label("https://example.com"), "example.com");
        assert_eq!(
            label("https://docs.example.com/guides/getting-started/installation/on-macos/advanced-setup"),
            "docs.example.com/…/advanced-setup"
        );
        assert!(label(&format!("https://a.io/{}", "x".repeat(80))).ends_with('…'));
    }

    #[test]
    fn markdown_links_use_the_selection_or_a_label() {
        assert_eq!(
            markdown("https://www.rust-lang.org/learn?utm_source=hn", None),
            "[rust-lang.org/learn](https://www.rust-lang.org/learn)"
        );
        assert_eq!(
            markdown("https://rust-lang.org", Some("Rust")),
            "[Rust](https://rust-lang.org)"
        );
    }
}

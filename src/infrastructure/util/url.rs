use crate::domain::error::tool_error::ToolError;
use reqwest::Url;
use std::net::IpAddr;

/// Parse and validate a URL for safe external fetching.
///
/// Rejects non-HTTP(S) schemes, localhost, and private/local IP addresses
/// to prevent SSRF attacks.
pub fn validate_external_url(raw_url: &str) -> Result<Url, ToolError> {
    let raw_url = raw_url.trim();
    if raw_url.is_empty() {
        return Err(ToolError::InvalidArguments(
            "missing or invalid 'url'".into(),
        ));
    }

    let url = Url::parse(raw_url)
        .map_err(|err| ToolError::InvalidArguments(format!("invalid 'url': {err}")))?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(ToolError::InvalidArguments(
            "'url' must use http or https".into(),
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| ToolError::InvalidArguments("'url' must include a host".into()))?;

    if host.eq_ignore_ascii_case("localhost") {
        return Err(ToolError::PermissionDenied(
            "localhost is not allowed".into(),
        ));
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        let blocked = match ip {
            IpAddr::V4(v4) => {
                v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_multicast()
                    || v4.is_unspecified()
            }
            IpAddr::V6(v6) => {
                v6.is_loopback()
                    || v6.is_multicast()
                    || v6.is_unspecified()
                    || v6.is_unique_local()
                    || v6.is_unicast_link_local()
            }
        };
        if blocked {
            return Err(ToolError::PermissionDenied(
                "private or local IP addresses are not allowed".into(),
            ));
        }
    }

    Ok(url)
}

//! Prefer header parsing (RFC 7240).
//!
//! Parses the HTTP Prefer header to extract PostgREST preferences.

use super::types::*;
use crate::error::{Error, Result};
use http::HeaderMap;

/// Parse Prefer header into Preferences struct.
pub fn parse_preferences(headers: &HeaderMap) -> Result<Preferences> {
    let mut prefs = Preferences::default();

    let prefer = match headers.get("prefer") {
        Some(v) => v.to_str().map_err(|_| Error::InvalidHeader("Prefer"))?,
        None => return Ok(prefs),
    };

    for pref in prefer.split(',').map(|s| s.trim()) {
        parse_preference(&mut prefs, pref);
    }

    Ok(prefs)
}

fn parse_preference(prefs: &mut Preferences, pref: &str) {
    let pref = pref.trim();

    // Handle key=value preferences
    if let Some((key, value)) = pref.split_once('=') {
        let key = key.trim();
        let value = value.trim().trim_matches('"');

        match key {
            "resolution" => {
                prefs.resolution = match value {
                    "merge-duplicates" => Some(PreferResolution::MergeDuplicates),
                    "ignore-duplicates" => Some(PreferResolution::IgnoreDuplicates),
                    _ => None,
                };
            }
            "return" => {
                prefs.representation = match value {
                    "representation" => PreferRepresentation::Full,
                    "headers-only" => PreferRepresentation::HeadersOnly,
                    "minimal" => PreferRepresentation::None,
                    _ => PreferRepresentation::None,
                };
            }
            "count" => {
                prefs.count = match value {
                    "exact" => Some(PreferCount::Exact),
                    "planned" => Some(PreferCount::Planned),
                    "estimated" => Some(PreferCount::Estimated),
                    _ => None,
                };
            }
            "tx" => {
                prefs.transaction = match value {
                    "commit" => PreferTransaction::Commit,
                    "rollback" => PreferTransaction::Rollback,
                    _ => PreferTransaction::Commit,
                };
            }
            "missing" => {
                prefs.missing = match value {
                    "default" => PreferMissing::ApplyDefaults,
                    "null" => PreferMissing::ApplyNulls,
                    _ => PreferMissing::ApplyDefaults,
                };
            }
            "handling" => {
                prefs.handling = match value {
                    "strict" => PreferHandling::Strict,
                    "lenient" => PreferHandling::Lenient,
                    _ => PreferHandling::Strict,
                };
            }
            "timezone" => {
                prefs.timezone = Some(value.to_string());
            }
            "max-affected" => {
                if let Ok(n) = value.parse::<i64>() {
                    prefs.max_affected = Some(n);
                }
            }
            _ => {
                prefs.invalid.push(pref.to_string());
            }
        }
        return;
    }

    // Handle standalone preferences
    match pref {
        "return=representation" => prefs.representation = PreferRepresentation::Full,
        "return=headers-only" => prefs.representation = PreferRepresentation::HeadersOnly,
        "return=minimal" => prefs.representation = PreferRepresentation::None,
        "count=exact" => prefs.count = Some(PreferCount::Exact),
        "count=planned" => prefs.count = Some(PreferCount::Planned),
        "count=estimated" => prefs.count = Some(PreferCount::Estimated),
        "resolution=merge-duplicates" => prefs.resolution = Some(PreferResolution::MergeDuplicates),
        "resolution=ignore-duplicates" => {
            prefs.resolution = Some(PreferResolution::IgnoreDuplicates)
        }
        "tx=commit" => prefs.transaction = PreferTransaction::Commit,
        "tx=rollback" => prefs.transaction = PreferTransaction::Rollback,
        "params=single-object" => {} // RPC parameter mode
        "params=multiple-objects" => {}
        _ => {
            prefs.invalid.push(pref.to_string());
        }
    }
}

/// Build Preference-Applied header from applied preferences.
pub fn preference_applied(prefs: &Preferences) -> Option<String> {
    let mut applied = Vec::new();

    if prefs.resolution.is_some() {
        let val = match prefs.resolution {
            Some(PreferResolution::MergeDuplicates) => "resolution=merge-duplicates",
            Some(PreferResolution::IgnoreDuplicates) => "resolution=ignore-duplicates",
            None => "",
        };
        if !val.is_empty() {
            applied.push(val);
        }
    }

    match prefs.representation {
        PreferRepresentation::Full => applied.push("return=representation"),
        PreferRepresentation::HeadersOnly => applied.push("return=headers-only"),
        PreferRepresentation::None => {}
    }

    if let Some(count) = &prefs.count {
        let val = match count {
            PreferCount::Exact => "count=exact",
            PreferCount::Planned => "count=planned",
            PreferCount::Estimated => "count=estimated",
        };
        applied.push(val);
    }

    match prefs.transaction {
        PreferTransaction::Rollback => applied.push("tx=rollback"),
        PreferTransaction::Commit => {}
    }

    if applied.is_empty() {
        None
    } else {
        Some(applied.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;

    fn headers_with_prefer(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("prefer", HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn test_parse_return_representation() {
        let headers = headers_with_prefer("return=representation");
        let prefs = parse_preferences(&headers).unwrap();
        assert_eq!(prefs.representation, PreferRepresentation::Full);
    }

    #[test]
    fn test_parse_count_exact() {
        let headers = headers_with_prefer("count=exact");
        let prefs = parse_preferences(&headers).unwrap();
        assert_eq!(prefs.count, Some(PreferCount::Exact));
    }

    #[test]
    fn test_parse_resolution() {
        let headers = headers_with_prefer("resolution=merge-duplicates");
        let prefs = parse_preferences(&headers).unwrap();
        assert_eq!(prefs.resolution, Some(PreferResolution::MergeDuplicates));
    }

    #[test]
    fn test_parse_multiple() {
        let headers = headers_with_prefer("return=representation, count=exact, tx=rollback");
        let prefs = parse_preferences(&headers).unwrap();
        assert_eq!(prefs.representation, PreferRepresentation::Full);
        assert_eq!(prefs.count, Some(PreferCount::Exact));
        assert_eq!(prefs.transaction, PreferTransaction::Rollback);
    }

    #[test]
    fn test_parse_timezone() {
        let headers = headers_with_prefer("timezone=America/New_York");
        let prefs = parse_preferences(&headers).unwrap();
        assert_eq!(prefs.timezone, Some("America/New_York".to_string()));
    }

    #[test]
    fn test_parse_max_affected() {
        let headers = headers_with_prefer("max-affected=100");
        let prefs = parse_preferences(&headers).unwrap();
        assert_eq!(prefs.max_affected, Some(100));
    }

    #[test]
    fn test_preference_applied() {
        let mut prefs = Preferences::default();
        prefs.representation = PreferRepresentation::Full;
        prefs.count = Some(PreferCount::Exact);

        let applied = preference_applied(&prefs).unwrap();
        assert!(applied.contains("return=representation"));
        assert!(applied.contains("count=exact"));
    }
}

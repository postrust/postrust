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

    // `handling=strict` asks to be told about preferences the server does not
    // implement rather than have them quietly dropped.
    if prefs.handling == PreferHandling::Strict && !prefs.invalid.is_empty() {
        return Err(Error::InvalidPreferences(prefs.invalid));
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
                if prefs.resolution.is_some() {
                    prefs.applied.push(format!("resolution={}", value));
                }
            }
            "return" => {
                prefs.representation = match value {
                    "representation" => PreferRepresentation::Full,
                    "headers-only" => PreferRepresentation::HeadersOnly,
                    "minimal" => PreferRepresentation::None,
                    _ => PreferRepresentation::None,
                };
                prefs.applied.push(format!("return={}", value));
            }
            "count" => {
                prefs.count = match value {
                    "exact" => Some(PreferCount::Exact),
                    "planned" => Some(PreferCount::Planned),
                    "estimated" => Some(PreferCount::Estimated),
                    _ => None,
                };
                if prefs.count.is_some() {
                    prefs.applied.push(format!("count={}", value));
                }
            }
            "tx" => {
                prefs.transaction = match value {
                    "commit" => PreferTransaction::Commit,
                    "rollback" => PreferTransaction::Rollback,
                    _ => PreferTransaction::Commit,
                };
                prefs.applied.push(format!("tx={}", value));
            }
            "missing" => {
                prefs.missing = match value {
                    "default" => PreferMissing::ApplyDefaults,
                    "null" => PreferMissing::ApplyNulls,
                    _ => PreferMissing::ApplyDefaults,
                };
                prefs.applied.push(format!("missing={}", value));
            }
            "handling" => {
                prefs.handling = match value {
                    "strict" => PreferHandling::Strict,
                    "lenient" => PreferHandling::Lenient,
                    _ => PreferHandling::Strict,
                };
                prefs.applied.push(format!("handling={}", value));
            }
            "timezone" => {
                prefs.timezone = Some(value.to_string());
                prefs.applied.push(format!("timezone={}", value));
            }
            // Understood, and applied where the RPC path reads the body.
            "params" => {}
            "max-affected" => {
                if let Ok(n) = value.parse::<i64>() {
                    prefs.max_affected = Some(n);
                    prefs.applied.push(format!("max-affected={}", n));
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

/// Build the `Preference-Applied` header.
///
/// Every preference the server understood, in the order it was sent. A client
/// reads it to find out which of what it asked for was honoured, so a
/// preference the server merely defaulted to has no business appearing.
pub fn preference_applied(prefs: &Preferences) -> Option<String> {
    match prefs.applied.is_empty() {
        true => None,
        false => Some(prefs.applied.join(", ")),
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
        let headers = headers_with_prefer("return=representation, count=exact");
        let prefs = parse_preferences(&headers).unwrap();

        let applied = preference_applied(&prefs).unwrap();
        assert!(applied.contains("return=representation"));
        assert!(applied.contains("count=exact"));
    }

    /// A preference the server merely defaulted to is not one it applied.
    #[test]
    fn preference_applied_reports_only_what_was_asked_for() {
        let headers = headers_with_prefer("handling=lenient");
        let prefs = parse_preferences(&headers).unwrap();

        assert_eq!(
            preference_applied(&prefs).as_deref(),
            Some("handling=lenient")
        );
        assert_eq!(preference_applied(&Preferences::default()), None);
    }
}

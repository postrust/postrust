//! Prefer header parsing (RFC 7240).
//!
//! Parses the HTTP Prefer header to extract PostgREST preferences.

use super::types::*;
use crate::error::{Error, Result};
use http::HeaderMap;

/// Parse Prefer header into Preferences struct.
pub fn parse_preferences(headers: &HeaderMap) -> Result<Preferences> {
    let mut prefs = Preferences::default();

    // A client may send one `Prefer` header with a comma-separated list, or
    // several headers, or both -- RFC 7240 allows all three and PostgREST's
    // own suite uses more than one. Reading only the first drops the rest.
    let mut seen_any = false;
    for value in headers.get_all("prefer") {
        let value = value.to_str().map_err(|_| Error::InvalidHeader("Prefer"))?;
        seen_any = true;
        for pref in value.split(',').map(|s| s.trim()) {
            parse_preference(&mut prefs, pref);
        }
    }

    if !seen_any {
        return Ok(prefs);
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
/// Only preferences that were asked for, and only those the request could
/// honour: `return=representation` says nothing on a read, and `missing`
/// nothing on a delete. PostgREST filters the same way and in this order,
/// and a client comparing the header against what it sent will notice.
///
/// `applied` records what was asked for, since a parsed field cannot say
/// whether its value was requested or is merely its default.
pub fn preference_applied(prefs: &Preferences, relevance: PreferenceScope) -> Option<String> {
    let asked = |name: &str| {
        prefs
            .applied
            .iter()
            .find(|pref| pref.starts_with(name))
            .cloned()
    };

    let mut values = Vec::new();
    if relevance.resolution {
        values.extend(asked("resolution="));
    }
    if relevance.missing {
        values.extend(asked("missing="));
    }
    if relevance.representation {
        values.extend(asked("return="));
    }
    values.extend(asked("count="));
    // `tx=` is deliberately absent. Ending the transaction the client's way
    // is something PostgREST does only where `db-tx-end` is configured to let
    // the request decide; by default the preference is not honoured, and so
    // is not reported. There is no such setting here and nothing reads
    // `Preferences::transaction`, which makes the answer the same one for a
    // stronger reason: a rollback that was asked for and never happened must
    // not come back described as applied.
    values.extend(asked("handling="));
    values.extend(asked("timezone="));
    // `max-affected` only has an effect under strict handling, so leniently
    // it was not applied.
    if relevance.max_affected && prefs.handling == PreferHandling::Strict {
        values.extend(asked("max-affected="));
    }

    match values.is_empty() {
        true => None,
        false => Some(values.join(", ")),
    }
}

/// Which preferences a given request could honour.
///
/// A preference that cannot apply to the request is not reported back, however
/// plainly it was asked for -- saying it was applied when it was ignored is
/// worse than saying nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct PreferenceScope {
    /// Insert, where duplicates can be resolved.
    pub resolution: bool,
    /// Insert or update, where a missing column can take a default.
    pub representation: bool,
    /// Insert or update.
    pub missing: bool,
    /// Update, delete or a function call.
    pub max_affected: bool,
}

impl PreferenceScope {
    /// What a read can honour: none of the write-only preferences.
    pub fn read() -> Self {
        Self::default()
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
        let writing = PreferenceScope {
            representation: true,
            ..PreferenceScope::read()
        };

        let applied = preference_applied(&prefs, writing).unwrap();
        assert!(applied.contains("return=representation"));
        assert!(applied.contains("count=exact"));
    }

    /// A preference the request could not act on is not reported as applied.
    #[test]
    fn preference_applied_leaves_out_what_a_read_cannot_honour() {
        let headers = headers_with_prefer("return=representation, count=exact");
        let prefs = parse_preferences(&headers).unwrap();

        assert_eq!(
            preference_applied(&prefs, PreferenceScope::read()).as_deref(),
            Some("count=exact")
        );
    }

    /// A preference nothing here acts on is not reported as applied, however
    /// plainly it was asked for. `tx=rollback` is parsed and then ignored --
    /// the transaction commits either way -- so a client told the preference
    /// was applied would believe its write had been undone.
    #[test]
    fn preference_applied_leaves_out_a_transaction_end_nothing_honours() {
        let headers = headers_with_prefer("tx=rollback");
        let prefs = parse_preferences(&headers).unwrap();

        assert_eq!(
            preference_applied(&prefs, PreferenceScope::read()),
            None,
            "tx= must not be reported while the transaction always commits"
        );

        let headers = headers_with_prefer("return=representation, tx=commit");
        let prefs = parse_preferences(&headers).unwrap();
        let writing = PreferenceScope {
            representation: true,
            ..PreferenceScope::read()
        };
        assert_eq!(
            preference_applied(&prefs, writing).as_deref(),
            Some("return=representation")
        );
    }

    /// A preference the server merely defaulted to is not one it applied.
    #[test]
    fn preference_applied_reports_only_what_was_asked_for() {
        let headers = headers_with_prefer("handling=lenient");
        let prefs = parse_preferences(&headers).unwrap();

        assert_eq!(
            preference_applied(&prefs, PreferenceScope::read()).as_deref(),
            Some("handling=lenient")
        );
        assert_eq!(
            preference_applied(&Preferences::default(), PreferenceScope::read()),
            None
        );
    }
}

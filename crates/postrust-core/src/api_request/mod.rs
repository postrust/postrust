//! API request parsing module.
//!
//! This module handles parsing HTTP requests into the domain-specific
//! `ApiRequest` type that can be used for query planning.

pub mod payload;
pub mod preferences;
pub mod query_params;
pub mod types;

pub use preferences::parse_preferences;
pub use query_params::{parse_query_params, value_is_filter};
pub use types::*;

use crate::error::{Error, Result};
use http::{Method, Request};
use percent_encoding::percent_decode_str;
use std::collections::{HashMap, HashSet};

/// Parse an HTTP request into an ApiRequest.
///
/// `max_rows` is the server-configured ceiling on returned rows
/// (`PGRST_MAX_ROWS`); pass `None` for no ceiling. It is applied when the read
/// plan resolves its range, so it caps requests that specify no `limit`.
pub fn parse_request<B>(
    req: &Request<B>,
    default_schema: &str,
    schemas: &[String],
    max_rows: Option<i64>,
) -> Result<ApiRequest>
where
    B: AsRef<[u8]>,
{
    let method = req.method();
    let path = req.uri().path();
    let query = req.uri().query().unwrap_or("");

    // Parse resource from path
    let resource = parse_resource(path)?;

    // Determine schema from headers or use default
    let (schema, negotiated_by_profile) = parse_schema(req, default_schema, schemas)?;

    // Parse action from method and resource
    let action = parse_action(method, &resource, &schema)?;

    // Parse query parameters. On a function *called over GET* the
    // unrecognized ones are arguments rather than malformed filters. Over
    // POST the arguments are in the body, so the query string is filters and
    // nothing else -- `POST /rpc/f?name=John` is a malformed filter, which is
    // what PostgREST says about it.
    let is_rpc = matches!(
        action,
        Action::Db(DbAction::Routine {
            invoke_method: InvokeMethod::InvRead { .. },
            ..
        }) | Action::RoutineInfo { .. }
    );
    let query_params = parse_query_params(query, is_rpc)?;

    // Parse preferences from Prefer header
    let preferences = parse_preferences(req.headers())?;

    // Parse Accept header for content negotiation
    let accept_media_types = parse_accept(req.headers())?;

    // Parse Content-Type header
    let content_media_type = parse_content_type(req.headers())?;

    // Parse Range header
    let top_level_range = parse_range(req.headers())?;

    // Extract headers and cookies for GUC passthrough
    let headers = extract_headers(req.headers());
    let cookies = extract_cookies(req.headers());

    Ok(ApiRequest {
        action,
        schema,
        payload: None, // Payload parsed separately
        query_params,
        accept_media_types,
        content_media_type,
        preferences,
        columns: HashSet::new(),
        top_level_range,
        range_map: HashMap::new(),
        max_rows,
        negotiated_by_profile,
        method: method.to_string(),
        path: path.to_string(),
        headers,
        cookies,
    })
}

/// Parse the resource from the URL path.
fn parse_resource(path: &str) -> Result<Resource> {
    let path = path.trim_start_matches('/');

    if path.is_empty() {
        return Ok(Resource::Schema);
    }

    // A trailing slash names the same resource: `/items/` is `/items`.
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return Ok(Resource::Schema);
    }

    // A name is the name the schema gave it, not the encoding a URL had to use
    // to carry it: `/%D9%85%D9%88%D8%A7%D8%B1%D8%AF` addresses a table called
    // `موارد`. Looking the encoded form up finds nothing, and says so using
    // the escape sequence rather than the name.
    let decoded = |segment: &str| -> String {
        percent_decode_str(segment)
            .decode_utf8()
            .map(|name| name.into_owned())
            .unwrap_or_else(|_| segment.to_string())
    };

    if let Some(func_name) = path.strip_prefix("rpc/") {
        if func_name.is_empty() {
            return Err(Error::InvalidPath("Empty function name".into()));
        }
        if func_name.contains('/') {
            return Err(Error::InvalidResourcePath);
        }
        return Ok(Resource::Routine(decoded(func_name)));
    }

    // A table is named by one segment. More than one names nothing at all --
    // there is no nesting in the API -- and reading only the first would
    // answer a request for `/first/second/third` with the contents of
    // `first`, which is not what was asked for.
    if path.contains('/') {
        return Err(Error::InvalidResourcePath);
    }

    Ok(Resource::Relation(decoded(path)))
}

/// Parse the schema from Accept-Profile or Content-Profile headers.
fn parse_schema<B>(
    req: &Request<B>,
    default_schema: &str,
    schemas: &[String],
) -> Result<(String, bool)> {
    // Check Accept-Profile header first (for reads)
    if let Some(profile) = req.headers().get("accept-profile") {
        let schema = profile
            .to_str()
            .map_err(|_| Error::InvalidHeader("Accept-Profile"))?;
        if !schemas.contains(&schema.to_string()) {
            return Err(Error::UnacceptableSchema {
                requested: schema.into(),
                exposed: schemas.to_vec(),
            });
        }
        return Ok((schema.to_string(), true));
    }

    // Check Content-Profile header (for writes)
    if let Some(profile) = req.headers().get("content-profile") {
        let schema = profile
            .to_str()
            .map_err(|_| Error::InvalidHeader("Content-Profile"))?;
        if !schemas.contains(&schema.to_string()) {
            return Err(Error::UnacceptableSchema {
                requested: schema.into(),
                exposed: schemas.to_vec(),
            });
        }
        return Ok((schema.to_string(), true));
    }

    Ok((default_schema.to_string(), false))
}

/// Parse the action from HTTP method and resource.
fn parse_action(method: &Method, resource: &Resource, schema: &str) -> Result<Action> {
    match (method, resource) {
        // Schema endpoints
        (&Method::GET, Resource::Schema) => Ok(Action::Db(DbAction::SchemaRead {
            schema: schema.to_string(),
            headers_only: false,
        })),
        (&Method::HEAD, Resource::Schema) => Ok(Action::Db(DbAction::SchemaRead {
            schema: schema.to_string(),
            headers_only: true,
        })),
        (&Method::OPTIONS, Resource::Schema) => Ok(Action::SchemaInfo),

        // Table/view endpoints
        (&Method::GET, Resource::Relation(name)) => Ok(Action::Db(DbAction::RelationRead {
            qi: QualifiedIdentifier::new(schema, name),
            headers_only: false,
        })),
        (&Method::HEAD, Resource::Relation(name)) => Ok(Action::Db(DbAction::RelationRead {
            qi: QualifiedIdentifier::new(schema, name),
            headers_only: true,
        })),
        (&Method::POST, Resource::Relation(name)) => Ok(Action::Db(DbAction::RelationMut {
            qi: QualifiedIdentifier::new(schema, name),
            mutation: Mutation::Create,
        })),
        (&Method::PATCH, Resource::Relation(name)) => Ok(Action::Db(DbAction::RelationMut {
            qi: QualifiedIdentifier::new(schema, name),
            mutation: Mutation::Update,
        })),
        (&Method::PUT, Resource::Relation(name)) => Ok(Action::Db(DbAction::RelationMut {
            qi: QualifiedIdentifier::new(schema, name),
            mutation: Mutation::SingleUpsert,
        })),
        (&Method::DELETE, Resource::Relation(name)) => Ok(Action::Db(DbAction::RelationMut {
            qi: QualifiedIdentifier::new(schema, name),
            mutation: Mutation::Delete,
        })),
        (&Method::OPTIONS, Resource::Relation(name)) => {
            Ok(Action::RelationInfo(QualifiedIdentifier::new(schema, name)))
        }

        // RPC endpoints
        (&Method::GET, Resource::Routine(name)) => Ok(Action::Db(DbAction::Routine {
            qi: QualifiedIdentifier::new(schema, name),
            invoke_method: InvokeMethod::InvRead {
                headers_only: false,
            },
        })),
        (&Method::HEAD, Resource::Routine(name)) => Ok(Action::Db(DbAction::Routine {
            qi: QualifiedIdentifier::new(schema, name),
            invoke_method: InvokeMethod::InvRead { headers_only: true },
        })),
        (&Method::POST, Resource::Routine(name)) => Ok(Action::Db(DbAction::Routine {
            qi: QualifiedIdentifier::new(schema, name),
            invoke_method: InvokeMethod::Inv,
        })),
        (&Method::OPTIONS, Resource::Routine(name)) => Ok(Action::RoutineInfo {
            qi: QualifiedIdentifier::new(schema, name),
            invoke_method: InvokeMethod::Inv,
        }),

        // Unsupported methods
        (_, Resource::Routine(_)) => Err(Error::InvalidRpcMethod(method.to_string())),
        _ => Err(Error::UnsupportedMethod(method.to_string())),
    }
}

/// Parse Accept header for content negotiation.
fn parse_accept(headers: &http::HeaderMap) -> Result<Vec<MediaType>> {
    if let Some(accept) = headers.get(http::header::ACCEPT) {
        let accept_str = accept
            .to_str()
            .map_err(|_| Error::InvalidHeader("Accept"))?;
        // A quality factor orders the list and says nothing about the type,
        // so it is dropped. The other parameters are not decoration: `;nulls=
        // stripped` is part of what `application/vnd.pgrst.array+json` *is*,
        // and cutting the entry at the semicolon threw it away along with the
        // `q=`.
        let types: Vec<MediaType> = accept_str
            .split(',')
            .map(str::trim)
            .map(|entry| {
                let kept: Vec<&str> = entry
                    .split(';')
                    .map(str::trim)
                    .filter(|part| !part.starts_with("q="))
                    .collect();
                parse_media_type(&kept.join(";"))
            })
            .collect();
        if types.is_empty() {
            return Ok(vec![MediaType::ApplicationJson]);
        }
        return Ok(types);
    }
    Ok(vec![MediaType::ApplicationJson])
}

/// Parse a single media type, parameters included.
///
/// Only the vendored types read their parameters; for everything else the
/// name alone is the type, so `application/json;charset=utf-8` is still JSON.
fn parse_media_type(s: &str) -> MediaType {
    let mut parts = s.split(';').map(str::trim);
    let base = parts.next().unwrap_or(s);
    // A parameter value may be quoted -- `for="application/json"` -- because
    // it is itself a media type and carries a `/`. The quotes delimit it and
    // are not part of it.
    let parameters: Vec<(String, &str)> = parts
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| {
            (
                key.trim().to_ascii_lowercase(),
                value.trim().trim_matches('"'),
            )
        })
        .collect();
    let param = |name: &str| {
        parameters
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
    };
    let given = |parameter: &str| match parameter.split_once('=') {
        Some((name, value)) => param(name) == Some(value),
        None => false,
    };

    match base {
        "application/json" => MediaType::ApplicationJson,
        "application/geo+json" => MediaType::GeoJson,
        "text/csv" => MediaType::TextCsv,
        "text/plain" => MediaType::TextPlain,
        "text/xml" => MediaType::TextXml,
        "application/openapi+json" => MediaType::OpenApi,
        "application/x-www-form-urlencoded" => MediaType::UrlEncoded,
        "application/octet-stream" => MediaType::OctetStream,
        "*/*" => MediaType::Any,
        // A plan is a plan *for* something, and that something is a media
        // type in its own right: read the same way, and named back the same
        // way. `for="application/vnd.pgrst.object"` is how the client says
        // "the plan for the query that would answer a singular request", and
        // it comes back out as `application/vnd.pgrst.object+json` because
        // that is the name that type has.
        s if s.starts_with("application/vnd.pgrst.plan") => {
            let format = match s.ends_with("+json") {
                true => PlanFormat::Json,
                false => PlanFormat::Text,
            };
            let requested = param("options").unwrap_or_default();
            // Named in a fixed order rather than the order they were asked
            // for, so that one set of options has one name.
            let options = [
                ("analyze", PlanOption::Analyze),
                ("verbose", PlanOption::Verbose),
                ("settings", PlanOption::Settings),
                ("buffers", PlanOption::Buffers),
                ("wal", PlanOption::Wal),
            ]
            .into_iter()
            .filter(|(name, _)| requested.split('|').any(|asked| asked == *name))
            .map(|(_, option)| option)
            .collect();
            MediaType::Plan {
                base: Box::new(parse_media_type(param("for").unwrap_or("application/json"))),
                format,
                options,
            }
        }
        s if s.starts_with("application/vnd.pgrst.object") => MediaType::SingularJson {
            nullable: given("nulls=null"),
            strip_nulls: given("nulls=stripped"),
        },
        s if s.starts_with("application/vnd.pgrst.array") => MediaType::ArrayJson {
            strip_nulls: given("nulls=stripped"),
        },
        other => MediaType::Other(other.to_string()),
    }
}

/// Parse Content-Type header.
fn parse_content_type(headers: &http::HeaderMap) -> Result<MediaType> {
    if let Some(ct) = headers.get(http::header::CONTENT_TYPE) {
        let ct_str = ct
            .to_str()
            .map_err(|_| Error::InvalidHeader("Content-Type"))?;
        return Ok(parse_media_type(ct_str.trim()));
    }
    Ok(MediaType::ApplicationJson)
}

/// Parse Range header for pagination.
///
/// `0-9`, `10-19` and `10-` are all ranges. Only the first of those was read
/// before, by matching the literal prefix `0-`; every other range was dropped
/// on the floor and the request answered with the whole relation. A client
/// paging with `Range: 1000-1999` was handed all of it and a `Content-Range`
/// saying so, which is the sort of disagreement a client discovers in
/// production rather than in a test.
///
/// A range that is not a range at all is left alone, as before: hyper accepts
/// header values this never has to make sense of, and the query parameters
/// remain the way to page.
fn parse_range(headers: &http::HeaderMap) -> Result<Range> {
    let Some(range) = headers.get(http::header::RANGE) else {
        return Ok(Range::default());
    };
    let range_str = range.to_str().map_err(|_| Error::InvalidHeader("Range"))?;

    // PostgREST writes the unit in `Range-Unit` and the bounds bare, but a
    // client following RFC 9110 puts the unit here. Both name the same rows.
    let bounds = range_str
        .split_once('=')
        .map_or(range_str, |(_unit, bounds)| bounds)
        .trim();

    let Some((start, end)) = bounds.split_once('-') else {
        return Ok(Range::default());
    };
    let Ok(start) = start.trim().parse::<i64>() else {
        return Ok(Range::default());
    };
    if start < 0 {
        return Ok(Range::default());
    }

    let end = end.trim();
    if end.is_empty() {
        // Open-ended: from here to the end of the relation.
        return Ok(Range::new(start, None));
    }
    let Ok(end) = end.parse::<i64>() else {
        return Ok(Range::default());
    };
    if end < start {
        return Err(Error::InvalidRange(
            "Requested range not satisfiable".into(),
        ));
    }

    Ok(Range::from_bounds(start, Some(end)))
}

/// Extract headers for GUC passthrough.
fn extract_headers(headers: &http::HeaderMap) -> indexmap::IndexMap<String, String> {
    headers
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
        .collect()
}

/// Extract cookies from Cookie header.
fn extract_cookies(headers: &http::HeaderMap) -> indexmap::IndexMap<String, String> {
    headers
        .get(http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            s.split(';')
                .filter_map(|cookie| {
                    let (key, value) = cookie.trim().split_once('=')?;

                    Some((key.to_string(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range_of(value: &str) -> Result<Range> {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::RANGE, value.parse().unwrap());
        parse_range(&headers)
    }

    /// Every range, not only the ones that begin at the first row.
    #[test]
    fn a_range_header_names_the_rows_it_says() {
        assert_eq!(range_of("0-9").unwrap(), Range::from_bounds(0, Some(9)));
        assert_eq!(range_of("5-9").unwrap(), Range::from_bounds(5, Some(9)));
        assert_eq!(range_of("10-19").unwrap(), Range::from_bounds(10, Some(19)));
        // Open-ended: from there to the end of the relation.
        assert_eq!(range_of("10-").unwrap(), Range::new(10, None));
        assert_eq!(range_of("0-").unwrap(), Range::new(0, None));
        // A unit, for a client that follows RFC 9110 rather than PostgREST.
        assert_eq!(
            range_of("items=5-9").unwrap(),
            Range::from_bounds(5, Some(9))
        );
    }

    /// A range whose end precedes its start names no rows at all.
    #[test]
    fn an_inverted_range_is_refused_rather_than_widened() {
        assert!(matches!(range_of("9-5"), Err(Error::InvalidRange(_))));
    }

    /// Anything that is not a range leaves paging to the query parameters,
    /// rather than being guessed at.
    #[test]
    fn a_header_that_is_not_a_range_is_left_alone() {
        for value in ["", "nonsense", "-5", "a-b", "5"] {
            assert_eq!(range_of(value).unwrap(), Range::default(), "{value:?}");
        }
    }

    /// A plan is named back with everything that made it that plan.
    ///
    /// The cases are PostgREST's own doctests for `decodeMediaType`, read
    /// back out through `toMime`.
    #[test]
    fn a_plan_media_type_is_named_back_in_full() {
        let named = |s: &str| parse_media_type(s).to_mime();

        assert_eq!(
            named("application/vnd.pgrst.plan+json"),
            "application/vnd.pgrst.plan+json; for=\"application/json\""
        );
        // The `for` type is read as a media type, so it is named by the name
        // that type has rather than the one the client wrote.
        assert_eq!(
            named("application/vnd.pgrst.plan+json; for=\"application/vnd.pgrst.object\""),
            "application/vnd.pgrst.plan+json; for=\"application/vnd.pgrst.object+json\""
        );
        assert_eq!(
            named("application/vnd.pgrst.plan; for=\"text/csv\""),
            "application/vnd.pgrst.plan+text; for=\"text/csv\""
        );
        // Options come back in a fixed order, not the order they were asked
        // for, and anything unrecognised is not an option.
        assert_eq!(
            named("application/vnd.pgrst.plan+json; options=verbose|analyze|nonsense"),
            "application/vnd.pgrst.plan+json; for=\"application/json\"; options=analyze|verbose"
        );
    }

    #[test]
    fn test_parse_resource() {
        assert_eq!(parse_resource("/").unwrap(), Resource::Schema);
        assert_eq!(
            parse_resource("/users").unwrap(),
            Resource::Relation("users".into())
        );
        assert_eq!(
            parse_resource("/rpc/my_func").unwrap(),
            Resource::Routine("my_func".into())
        );
    }

    #[test]
    fn test_parse_media_type() {
        assert_eq!(
            parse_media_type("application/json"),
            MediaType::ApplicationJson
        );
        assert_eq!(parse_media_type("text/csv"), MediaType::TextCsv);
        assert_eq!(parse_media_type("*/*"), MediaType::Any);
    }
}

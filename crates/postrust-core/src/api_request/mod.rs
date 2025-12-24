//! API request parsing module.
//!
//! This module handles parsing HTTP requests into the domain-specific
//! `ApiRequest` type that can be used for query planning.

pub mod types;
pub mod query_params;
pub mod payload;
pub mod preferences;

pub use types::*;
pub use query_params::parse_query_params;
pub use preferences::parse_preferences;

use crate::error::{Error, Result};
use http::{Method, Request};
use std::collections::{HashMap, HashSet};

/// Parse an HTTP request into an ApiRequest.
pub fn parse_request<B>(
    req: &Request<B>,
    default_schema: &str,
    schemas: &[String],
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

    // Parse query parameters
    let query_params = parse_query_params(query)?;

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

    if let Some(func_name) = path.strip_prefix("rpc/") {
        if func_name.is_empty() {
            return Err(Error::InvalidPath("Empty function name".into()));
        }
        return Ok(Resource::Routine(func_name.to_string()));
    }

    // Table/view name is the first path segment
    let name = path.split('/').next().unwrap_or(path);
    if name.is_empty() {
        return Err(Error::InvalidPath("Empty resource name".into()));
    }

    Ok(Resource::Relation(name.to_string()))
}

/// Parse the schema from Accept-Profile or Content-Profile headers.
fn parse_schema<B>(
    req: &Request<B>,
    default_schema: &str,
    schemas: &[String],
) -> Result<(String, bool)> {
    // Check Accept-Profile header first (for reads)
    if let Some(profile) = req.headers().get("accept-profile") {
        let schema = profile.to_str().map_err(|_| Error::InvalidHeader("Accept-Profile"))?;
        if !schemas.contains(&schema.to_string()) {
            return Err(Error::UnacceptableSchema(schema.into()));
        }
        return Ok((schema.to_string(), true));
    }

    // Check Content-Profile header (for writes)
    if let Some(profile) = req.headers().get("content-profile") {
        let schema = profile.to_str().map_err(|_| Error::InvalidHeader("Content-Profile"))?;
        if !schemas.contains(&schema.to_string()) {
            return Err(Error::UnacceptableSchema(schema.into()));
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
            invoke_method: InvokeMethod::InvRead { headers_only: false },
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
        _ => Err(Error::UnsupportedMethod(method.to_string())),
    }
}

/// Parse Accept header for content negotiation.
fn parse_accept(headers: &http::HeaderMap) -> Result<Vec<MediaType>> {
    if let Some(accept) = headers.get(http::header::ACCEPT) {
        let accept_str = accept.to_str().map_err(|_| Error::InvalidHeader("Accept"))?;
        // Simple parsing - full implementation would handle quality factors
        let types: Vec<MediaType> = accept_str
            .split(',')
            .map(|s| s.trim())
            .map(|s| s.split(';').next().unwrap_or(s).trim())
            .map(parse_media_type)
            .collect();
        if types.is_empty() {
            return Ok(vec![MediaType::ApplicationJson]);
        }
        return Ok(types);
    }
    Ok(vec![MediaType::ApplicationJson])
}

/// Parse a single media type string.
fn parse_media_type(s: &str) -> MediaType {
    match s {
        "application/json" => MediaType::ApplicationJson,
        "application/geo+json" => MediaType::GeoJson,
        "text/csv" => MediaType::TextCsv,
        "text/plain" => MediaType::TextPlain,
        "text/xml" => MediaType::TextXml,
        "application/openapi+json" => MediaType::OpenApi,
        "application/x-www-form-urlencoded" => MediaType::UrlEncoded,
        "application/octet-stream" => MediaType::OctetStream,
        "*/*" => MediaType::Any,
        s if s.starts_with("application/vnd.pgrst.object") => {
            MediaType::SingularJson { nullable: s.contains("nulls=null") }
        }
        s if s.starts_with("application/vnd.pgrst.array") => MediaType::ArrayJsonStrip,
        other => MediaType::Other(other.to_string()),
    }
}

/// Parse Content-Type header.
fn parse_content_type(headers: &http::HeaderMap) -> Result<MediaType> {
    if let Some(ct) = headers.get(http::header::CONTENT_TYPE) {
        let ct_str = ct.to_str().map_err(|_| Error::InvalidHeader("Content-Type"))?;
        let media_type = ct_str.split(';').next().unwrap_or(ct_str).trim();
        return Ok(parse_media_type(media_type));
    }
    Ok(MediaType::ApplicationJson)
}

/// Parse Range header for pagination.
fn parse_range(headers: &http::HeaderMap) -> Result<Range> {
    if let Some(range) = headers.get(http::header::RANGE) {
        let range_str = range.to_str().map_err(|_| Error::InvalidHeader("Range"))?;
        // Parse "0-9" or "10-" format
        if let Some(range_value) = range_str.strip_prefix("0-") {
            if range_value.is_empty() {
                return Ok(Range::new(0, None));
            }
            if let Ok(end) = range_value.parse::<i64>() {
                return Ok(Range::from_bounds(0, Some(end)));
            }
        }
        // More complex range parsing would go here
    }
    Ok(Range::default())
}

/// Extract headers for GUC passthrough.
fn extract_headers(headers: &http::HeaderMap) -> indexmap::IndexMap<String, String> {
    headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str().ok().map(|v| (k.to_string(), v.to_string()))
        })
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
                    let mut parts = cookie.trim().splitn(2, '=');
                    let key = parts.next()?;
                    let value = parts.next()?;
                    Some((key.to_string(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(parse_media_type("application/json"), MediaType::ApplicationJson);
        assert_eq!(parse_media_type("text/csv"), MediaType::TextCsv);
        assert_eq!(parse_media_type("*/*"), MediaType::Any);
    }
}

//! Query parameter parsing using nom.
//!
//! Parses URL query strings into structured filter, select, order, and range data.
//! Mirrors PostgREST's QueryParams.hs parsing logic.

use super::types::*;
use crate::error::{Error, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until, take_while1},
    character::complete::{char, digit1},
    combinator::{map, opt, value},
    multi::{many0, separated_list0},
    sequence::preceded,
    IResult,
};
use percent_encoding::percent_decode_str;

/// Parse a query string into QueryParams.
pub fn parse_query_params(query: &str) -> Result<QueryParams> {
    let mut params = QueryParams::default();

    if query.is_empty() {
        return Ok(params);
    }

    // Sort parameters for canonical form
    let mut pairs: Vec<(&str, &str)> = query
        .split('&')
        .filter_map(|pair| {
            let mut parts = pair.splitn(2, '=');
            Some((parts.next()?, parts.next().unwrap_or("")))
        })
        .collect();
    pairs.sort_by_key(|(k, _)| *k);
    params.canonical = pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join("&");

    for (key, value) in pairs {
        let decoded_value = percent_decode_str(value)
            .decode_utf8()
            .map_err(|_| Error::InvalidQueryParam(key.into()))?
            .to_string();

        match key {
            "select" => {
                params.select = parse_select(&decoded_value)?;
            }
            "order" => {
                let (path, terms) = parse_order_param(&decoded_value)?;
                params.order.push((path, terms));
            }
            "limit" => {
                let limit: i64 = decoded_value
                    .parse()
                    .map_err(|_| Error::InvalidQueryParam("limit".into()))?;
                params.ranges.entry(String::new()).or_default().limit = Some(limit);
            }
            "offset" => {
                let offset: i64 = decoded_value
                    .parse()
                    .map_err(|_| Error::InvalidQueryParam("offset".into()))?;
                params.ranges.entry(String::new()).or_default().offset = offset;
            }
            "columns" => {
                params.columns = Some(
                    decoded_value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect(),
                );
            }
            "on_conflict" => {
                params.on_conflict = Some(
                    decoded_value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect(),
                );
            }
            "and" | "or" => {
                let logic = parse_logic_param(key, &decoded_value)?;
                params.logic.push((vec![], logic));
            }
            key if !key.starts_with('_') => {
                // Filter parameter
                let (path, filter) = parse_filter_param(key, &decoded_value)?;
                if path.is_empty() {
                    params.filter_fields.insert(filter.field.name.clone());
                    params.filters_root.push(filter);
                } else {
                    params.filters.push((path, filter));
                }
            }
            _ => {
                // RPC parameters (anything else)
                params.params.push((key.to_string(), decoded_value));
            }
        }
    }

    Ok(params)
}

// ============================================================================
// Select Parsing
// ============================================================================

/// Parse the `select` parameter value.
pub fn parse_select(input: &str) -> Result<Vec<SelectItem>> {
    if input.is_empty() {
        return Ok(vec![]);
    }

    match parse_select_items(input) {
        Ok((_, items)) => Ok(items),
        Err(_) => Err(Error::InvalidQueryParam("select".into())),
    }
}

fn parse_select_items(input: &str) -> IResult<&str, Vec<SelectItem>> {
    separated_list0(char(','), parse_select_item)(input)
}

fn parse_select_item(input: &str) -> IResult<&str, SelectItem> {
    alt((
        parse_spread_relation,
        parse_relation_select,
        parse_field_select,
    ))(input)
}

/// Parse spread relation: `...relation`
fn parse_spread_relation(input: &str) -> IResult<&str, SelectItem> {
    let (input, _) = tag("...")(input)?;
    let (input, relation) = parse_identifier(input)?;
    let (input, hint) = opt(preceded(char('!'), parse_identifier))(input)?;
    let (input, join_type) = opt(preceded(char('!'), parse_join_type))(input)?;

    Ok((
        input,
        SelectItem::SpreadRelation {
            relation: relation.to_string(),
            hint: hint.map(|s| s.to_string()),
            join_type,
        },
    ))
}

/// Parse relation with embedded select: `relation(select_items)`
fn parse_relation_select(input: &str) -> IResult<&str, SelectItem> {
    let (input, name) = parse_identifier(input)?;
    let (input, alias) = opt(preceded(char(':'), parse_identifier))(input)?;
    let (input, hint) = opt(preceded(char('!'), parse_identifier))(input)?;
    let (input, join_type) = opt(preceded(char('!'), parse_join_type))(input)?;
    let (input, _) = char('(')(input)?;
    let (input, _nested) = take_until(")")(input)?;
    let (input, _) = char(')')(input)?;

    Ok((
        input,
        SelectItem::Relation {
            relation: name.to_string(),
            alias: alias.map(|s| s.to_string()),
            hint: hint.map(|s| s.to_string()),
            join_type,
        },
    ))
}

/// Parse field select: `field`, `field::cast`, `field:alias`, `agg(field)`
fn parse_field_select(input: &str) -> IResult<&str, SelectItem> {
    // Check for aggregate function
    let (input, aggregate) = opt(parse_aggregate_prefix)(input)?;

    let (input, name) = parse_identifier(input)?;
    let (input, json_path) = parse_json_path(input)?;

    // Close aggregate if present
    let (input, aggregate_cast) = if aggregate.is_some() {
        let (input, _) = char(')')(input)?;
        let (input, cast) = opt(preceded(tag("::"), parse_identifier))(input)?;
        (input, cast.map(|s| s.to_string()))
    } else {
        (input, None)
    };

    let (input, cast) = if aggregate.is_none() {
        opt(preceded(tag("::"), parse_identifier))(input)?
    } else {
        (input, None)
    };

    let (input, alias) = opt(preceded(char(':'), parse_identifier))(input)?;

    Ok((
        input,
        SelectItem::Field {
            field: Field {
                name: name.to_string(),
                json_path,
            },
            aggregate,
            aggregate_cast,
            cast: cast.map(|s| s.to_string()),
            alias: alias.map(|s| s.to_string()),
        },
    ))
}

fn parse_aggregate_prefix(input: &str) -> IResult<&str, AggregateFunction> {
    alt((
        value(AggregateFunction::Sum, tag("sum(")),
        value(AggregateFunction::Avg, tag("avg(")),
        value(AggregateFunction::Max, tag("max(")),
        value(AggregateFunction::Min, tag("min(")),
        value(AggregateFunction::Count, tag("count(")),
    ))(input)
}

fn parse_join_type(input: &str) -> IResult<&str, JoinType> {
    alt((
        value(JoinType::Inner, tag("inner")),
        value(JoinType::Left, tag("left")),
    ))(input)
}

// ============================================================================
// Filter Parsing
// ============================================================================

/// Parse a filter parameter (key=value where key is a field name).
fn parse_filter_param(key: &str, value: &str) -> Result<(EmbedPath, Filter)> {
    // Parse the key for embedded path: rel.field or field
    let (path, field_name) = parse_filter_key(key)?;

    // Parse the value for operator and operand
    let op_expr = parse_filter_value(value)?;

    let filter = Filter::new(Field::simple(field_name), op_expr);
    Ok((path, filter))
}

/// Parse a filter key into path and field name.
fn parse_filter_key(key: &str) -> Result<(EmbedPath, String)> {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        return Err(Error::InvalidQueryParam(key.into()));
    }

    if parts.len() == 1 {
        return Ok((vec![], parts[0].to_string()));
    }

    let path: Vec<String> = parts[..parts.len() - 1].iter().map(|s| s.to_string()).collect();
    let field = parts.last().unwrap().to_string();
    Ok((path, field))
}

/// Parse filter value: `operator.value` or `not.operator.value`
fn parse_filter_value(value: &str) -> Result<OpExpr> {
    let (value, negated) = if let Some(rest) = value.strip_prefix("not.") {
        (rest, true)
    } else {
        (value, false)
    };

    let operation = parse_operation(value)?;
    Ok(OpExpr { negated, operation })
}

/// Parse an operation: `eq.value`, `in.(a,b,c)`, `is.null`, etc.
fn parse_operation(value: &str) -> Result<Operation> {
    // Try each operator pattern
    if let Some(rest) = value.strip_prefix("eq.") {
        return Ok(Operation::Quant {
            op: QuantOperator::Equal,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("neq.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::NotEqual,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("gt.") {
        return Ok(Operation::Quant {
            op: QuantOperator::GreaterThan,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("gte.") {
        return Ok(Operation::Quant {
            op: QuantOperator::GreaterThanEqual,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("lt.") {
        return Ok(Operation::Quant {
            op: QuantOperator::LessThan,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("lte.") {
        return Ok(Operation::Quant {
            op: QuantOperator::LessThanEqual,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("like.") {
        return Ok(Operation::Quant {
            op: QuantOperator::Like,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("ilike.") {
        return Ok(Operation::Quant {
            op: QuantOperator::ILike,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("match.") {
        return Ok(Operation::Quant {
            op: QuantOperator::Match,
            quantifier: None,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("imatch.") {
        return Ok(Operation::Quant {
            op: QuantOperator::IMatch,
            quantifier: None,
            value: rest.to_string(),
        });
    }

    // Array/Range operators
    if let Some(rest) = value.strip_prefix("cs.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::Contains,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("cd.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::Contained,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("ov.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::Overlap,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("sl.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::StrictlyLeft,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("sr.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::StrictlyRight,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("nxr.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::NotExtendsRight,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("nxl.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::NotExtendsLeft,
            value: rest.to_string(),
        });
    }
    if let Some(rest) = value.strip_prefix("adj.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::Adjacent,
            value: rest.to_string(),
        });
    }

    // IN operator
    if let Some(rest) = value.strip_prefix("in.") {
        let values = parse_in_list(rest)?;
        return Ok(Operation::In(values));
    }

    // IS operator
    if let Some(rest) = value.strip_prefix("is.") {
        let is_val = match rest {
            "null" => IsValue::Null,
            "true" => IsValue::True,
            "false" => IsValue::False,
            "unknown" => IsValue::Unknown,
            _ => return Err(Error::InvalidQueryParam(format!("is.{}", rest))),
        };
        return Ok(Operation::Is(is_val));
    }

    // IS DISTINCT FROM
    if let Some(rest) = value.strip_prefix("isdistinct.") {
        return Ok(Operation::IsDistinctFrom(rest.to_string()));
    }

    // Full-text search
    if let Some(rest) = value.strip_prefix("fts") {
        return parse_fts(FtsOperator::Fts, rest);
    }
    if let Some(rest) = value.strip_prefix("plfts") {
        return parse_fts(FtsOperator::Plain, rest);
    }
    if let Some(rest) = value.strip_prefix("phfts") {
        return parse_fts(FtsOperator::Phrase, rest);
    }
    if let Some(rest) = value.strip_prefix("wfts") {
        return parse_fts(FtsOperator::Websearch, rest);
    }

    Err(Error::InvalidQueryParam(value.into()))
}

/// Parse IN list: `(a,b,c)` -> vec!["a", "b", "c"]
fn parse_in_list(value: &str) -> Result<Vec<String>> {
    let value = value
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| Error::InvalidQueryParam(format!("in.{}", value)))?;

    Ok(value.split(',').map(|s| s.trim().to_string()).collect())
}

/// Parse FTS operation: `(language).query` or `.query`
fn parse_fts(op: FtsOperator, rest: &str) -> Result<Operation> {
    if let Some(rest) = rest.strip_prefix('(') {
        // Has language specifier
        let (lang, query) = rest
            .split_once(").")
            .ok_or_else(|| Error::InvalidQueryParam(format!("fts{}", rest)))?;
        return Ok(Operation::Fts {
            op,
            language: Some(lang.to_string()),
            value: query.to_string(),
        });
    }

    let query = rest
        .strip_prefix('.')
        .ok_or_else(|| Error::InvalidQueryParam(format!("fts{}", rest)))?;
    Ok(Operation::Fts {
        op,
        language: None,
        value: query.to_string(),
    })
}

// ============================================================================
// Order Parsing
// ============================================================================

/// Parse order parameter: `col.desc.nullsfirst,col2.asc`
fn parse_order_param(value: &str) -> Result<(EmbedPath, Vec<OrderTerm>)> {
    let terms: Vec<OrderTerm> = value
        .split(',')
        .map(|s| parse_order_term(s.trim()))
        .collect::<Result<Vec<_>>>()?;
    Ok((vec![], terms))
}

fn parse_order_term(value: &str) -> Result<OrderTerm> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.is_empty() {
        return Err(Error::InvalidQueryParam("order".into()));
    }

    let field_name = parts[0];
    let mut direction = None;
    let mut nulls = None;

    for part in &parts[1..] {
        match *part {
            "asc" => direction = Some(OrderDirection::Asc),
            "desc" => direction = Some(OrderDirection::Desc),
            "nullsfirst" => nulls = Some(OrderNulls::First),
            "nullslast" => nulls = Some(OrderNulls::Last),
            _ => {}
        }
    }

    Ok(OrderTerm::Field {
        field: Field::simple(field_name),
        direction,
        nulls,
    })
}

// ============================================================================
// Logic Tree Parsing
// ============================================================================

/// Parse `and` or `or` parameter: `(filter1,filter2)`
fn parse_logic_param(op: &str, value: &str) -> Result<LogicTree> {
    let logic_op = match op {
        "and" => LogicOperator::And,
        "or" => LogicOperator::Or,
        _ => return Err(Error::InvalidQueryParam(op.into())),
    };

    // Parse nested filters: (field.op.value,field2.op.value)
    let value = value
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| Error::InvalidQueryParam(format!("{}={}", op, value)))?;

    let children: Vec<LogicTree> = value
        .split(',')
        .map(|s| {
            let (key, val) = s
                .split_once('.')
                .ok_or_else(|| Error::InvalidQueryParam(s.into()))?;
            let (_, filter) = parse_filter_param(key, val)?;
            Ok(LogicTree::Stmt(filter))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(LogicTree::Expr {
        negated: false,
        op: logic_op,
        children,
    })
}

// ============================================================================
// Helper Parsers
// ============================================================================

fn parse_identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_')(input)
}

fn parse_json_path(input: &str) -> IResult<&str, JsonPath> {
    many0(alt((parse_arrow, parse_double_arrow)))(input)
}

fn parse_arrow(input: &str) -> IResult<&str, JsonOperation> {
    let (input, _) = tag("->")(input)?;
    let (input, operand) = alt((
        map(digit1, |s: &str| {
            JsonOperand::Idx(s.parse().unwrap_or(0))
        }),
        map(parse_identifier, |s| JsonOperand::Key(s.to_string())),
    ))(input)?;
    Ok((input, JsonOperation::Arrow(operand)))
}

fn parse_double_arrow(input: &str) -> IResult<&str, JsonOperation> {
    let (input, _) = tag("->>")(input)?;
    let (input, operand) = alt((
        map(digit1, |s: &str| {
            JsonOperand::Idx(s.parse().unwrap_or(0))
        }),
        map(parse_identifier, |s| JsonOperand::Key(s.to_string())),
    ))(input)?;
    Ok((input, JsonOperation::DoubleArrow(operand)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_filter() {
        let params = parse_query_params("name=eq.John").unwrap();
        assert_eq!(params.filters_root.len(), 1);
        assert_eq!(params.filters_root[0].field.name, "name");
    }

    #[test]
    fn test_parse_negated_filter() {
        let params = parse_query_params("status=not.eq.active").unwrap();
        assert!(params.filters_root[0].op_expr.negated);
    }

    #[test]
    fn test_parse_in_filter() {
        let params = parse_query_params("id=in.(1,2,3)").unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::In(values) => {
                assert_eq!(values, &vec!["1", "2", "3"]);
            }
            _ => panic!("Expected In operation"),
        }
    }

    #[test]
    fn test_parse_is_null() {
        let params = parse_query_params("deleted_at=is.null").unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::Is(IsValue::Null) => {}
            _ => panic!("Expected Is Null"),
        }
    }

    #[test]
    fn test_parse_order() {
        let params = parse_query_params("order=name.asc,age.desc.nullslast").unwrap();
        assert_eq!(params.order.len(), 1);
        let (_, terms) = &params.order[0];
        assert_eq!(terms.len(), 2);
    }

    #[test]
    fn test_parse_limit_offset() {
        let params = parse_query_params("limit=10&offset=20").unwrap();
        let range = params.ranges.get("").unwrap();
        assert_eq!(range.limit, Some(10));
        assert_eq!(range.offset, 20);
    }

    #[test]
    fn test_parse_select() {
        let items = parse_select("id,name,orders(id,amount)").unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn test_parse_fts() {
        let params = parse_query_params("content=fts(english).search+term").unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::Fts { op, language, value } => {
                assert_eq!(*op, FtsOperator::Fts);
                assert_eq!(language.as_deref(), Some("english"));
                assert_eq!(value, "search+term");
            }
            _ => panic!("Expected FTS operation"),
        }
    }
}

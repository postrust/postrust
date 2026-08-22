//! Query parameter parsing using nom.
//!
//! Parses URL query strings into structured filter, select, order, and range data.
//! Mirrors PostgREST's QueryParams.hs parsing logic.

use super::types::*;
use crate::error::{Error, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{char, digit1},
    combinator::{map, opt, value},
    multi::{many0, separated_list0},
    sequence::preceded,
    IResult,
};
use percent_encoding::percent_decode_str;

/// Parse a query string into QueryParams.
///
/// `is_rpc` changes what an unrecognized key means. On a table, `a=2` is a
/// malformed filter and an error, because a filter value must carry an
/// operator. On a function, it is an argument -- so every such key is also
/// recorded as a candidate argument, and one that fails to parse as a filter
/// is no longer fatal. Which candidates are really arguments is settled at
/// plan time, against the parameters the routine actually declares; a
/// well-formed filter such as `id=eq.1` stays available to filter the result.
pub fn parse_query_params(query: &str, is_rpc: bool) -> Result<QueryParams> {
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

        // The key is decoded as well as the value. A filter on a JSON path
        // arrives as `data-%3E%3Eb` from any client that escapes `>`, and
        // would otherwise be looked up as a column of that literal name.
        let decoded_key = percent_decode_str(key)
            .decode_utf8()
            .map_err(|_| Error::InvalidQueryParam(key.into()))?
            .to_string();
        let key: &str = &decoded_key;

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
                if is_rpc {
                    params.params.push((key.to_string(), decoded_value.clone()));
                }

                // Filter parameter
                let parsed = parse_filter_param(key, &decoded_value);
                let (path, filter) = match parsed {
                    Ok(parsed) => parsed,
                    // On a function, a key that isn't a filter is just an
                    // argument, which was recorded above.
                    Err(_) if is_rpc => continue,
                    Err(e) => return Err(e),
                };

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
        // Before relations: `count()` is spelled exactly like an embed of a
        // relation named `count` with an empty selection.
        parse_bare_aggregate,
        parse_spread_relation,
        parse_relation_select,
        parse_field_select,
    ))(input)
}

/// Parse a field-less aggregate: `count()`, `cnt:count()`, `count()::text`.
///
/// Only `count` is meaningful without a field -- the others have nothing to
/// sum -- which is also what keeps this from swallowing embeds.
fn parse_bare_aggregate(input: &str) -> IResult<&str, SelectItem> {
    let (input, alias) = opt(parse_alias_prefix)(input)?;
    let (input, _) = tag("count()")(input)?;
    let (input, cast) = opt(preceded(tag("::"), parse_identifier))(input)?;

    Ok((
        input,
        SelectItem::Field {
            // An empty name marks the `COUNT(*)` form: there is no column to
            // resolve, and nothing to group by.
            field: Field::simple(""),
            aggregate: Some(AggregateFunction::Count),
            aggregate_cast: cast.map(|s| s.to_string()),
            cast: None,
            alias: alias.map(|s| s.to_string()),
        },
    ))
}

/// Parse an aggregate applied to a field: the `.sum()` of `amount.sum()`.
fn parse_aggregate_suffix(input: &str) -> IResult<&str, AggregateFunction> {
    let (input, _) = char('.')(input)?;
    let (input, function) = alt((
        value(AggregateFunction::Sum, tag("sum")),
        value(AggregateFunction::Avg, tag("avg")),
        value(AggregateFunction::Max, tag("max")),
        value(AggregateFunction::Min, tag("min")),
        value(AggregateFunction::Count, tag("count")),
    ))(input)?;
    let (input, _) = tag("()")(input)?;
    Ok((input, function))
}

/// Parse spread relation: `...relation(cols)`
fn parse_spread_relation(input: &str) -> IResult<&str, SelectItem> {
    let (input, _) = tag("...")(input)?;
    let (input, relation) = parse_identifier(input)?;
    let (input, (hint, join_type)) = parse_relation_modifiers(input)?;
    // The parentheses are as much a part of the syntax here as they are for a
    // plain embed. Leaving them for the caller to trip over meant the column
    // list was silently discarded along with the rest of the select.
    let (input, _) = char('(')(input)?;
    let (input, nested) = parse_nested_select(input)?;
    let (input, _) = char(')')(input)?;

    Ok((
        input,
        SelectItem::SpreadRelation {
            relation: relation.to_string(),
            hint,
            join_type,
            select: nested,
        },
    ))
}

/// Parse the contents of a relation's parentheses.
///
/// Scans to the parenthesis that closes the group -- tracking nesting, so
/// `books(chapters(id))` is not truncated at the first `)` -- then parses the
/// contents as a select list.
fn parse_nested_select(input: &str) -> IResult<&str, Vec<SelectItem>> {
    let mut depth = 0usize;
    let mut end = input.len();

    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                if depth == 0 {
                    end = idx;
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    let (body, rest) = input.split_at(end);

    if body.is_empty() {
        return Ok((rest, Vec::new()));
    }

    match parse_select_items(body) {
        Ok(("", items)) => Ok((rest, items)),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

/// Parse relation with embedded select: `relation(select_items)`
fn parse_relation_select(input: &str) -> IResult<&str, SelectItem> {
    // As with fields, the alias precedes what it names: `c:clients(*)`.
    let (input, alias) = opt(parse_alias_prefix)(input)?;
    let (input, name) = parse_identifier(input)?;
    let (input, (hint, join_type)) = parse_relation_modifiers(input)?;
    // The nested selection is parsed rather than skipped: it says which
    // columns of the related resource to return, and may embed further
    // relations of its own.
    let (input, _) = char('(')(input)?;
    let (input, nested) = parse_nested_select(input)?;
    let (input, _) = char(')')(input)?;

    Ok((
        input,
        SelectItem::Relation {
            relation: name.to_string(),
            alias: alias.map(|s| s.to_string()),
            hint,
            join_type,
            select: nested,
        },
    ))
}

/// Parse field select: `field`, `field::cast`, `field:alias`, `agg(field)`
fn parse_field_select(input: &str) -> IResult<&str, SelectItem> {
    // `alias:expression` -- the alias comes first, as in `myId:id` or
    // `total:sum(amount)`. Only a name immediately followed by `:` is one;
    // anything else backtracks and is read as the expression itself.
    let (input, alias) = opt(parse_alias_prefix)(input)?;

    let (input, name) = parse_identifier(input)?;
    let (input, json_path) = parse_json_path(input)?;

    // A cast on the column binds before the aggregate: `key::integer.sum()`
    // sums integers, where `.sum()::text` renders the sum as text.
    let (input, cast) = opt(preceded(tag("::"), parse_identifier))(input)?;
    let (input, aggregate) = opt(parse_aggregate_suffix)(input)?;
    let (input, aggregate_cast) = if aggregate.is_some() {
        let (input, cast) = opt(preceded(tag("::"), parse_identifier))(input)?;
        (input, cast.map(|s| s.to_string()))
    } else {
        (input, None)
    };

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

/// Parse the `!`-prefixed modifiers on an embedded relation.
///
/// A relation may carry a disambiguating hint, a join type, or both, in either
/// order: `books!author(*)`, `books!inner(*)`, `books!author!inner(*)`. They
/// cannot be told apart by position, only by spelling -- `inner` and `left`
/// are join types and anything else names a foreign key or table -- so each
/// modifier is read in turn and classified.
/// An `alias:` prefix, distinguished from the `::` of a cast.
///
/// `myId:id` names the column; `id::text` casts it. Both start with an
/// identifier followed by a colon, so the second colon is what tells them
/// apart -- without this check `id::text` would be read as an alias `id`
/// applied to nothing.
fn parse_alias_prefix(input: &str) -> IResult<&str, &str> {
    let (rest, name) = parse_identifier(input)?;
    let (rest, _) = char(':')(rest)?;

    if rest.starts_with(':') {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }

    Ok((rest, name))
}

fn parse_relation_modifiers(input: &str) -> IResult<&str, (Option<String>, Option<JoinType>)> {
    let (input, modifiers) = many0(preceded(char('!'), parse_identifier))(input)?;

    let mut hint = None;
    let mut join_type = None;
    for modifier in modifiers {
        match modifier {
            "inner" => join_type = Some(JoinType::Inner),
            "left" => join_type = Some(JoinType::Left),
            other => hint = Some(other.to_string()),
        }
    }

    Ok((input, (hint, join_type)))
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

    let filter = Filter::new(split_json_path(&field_name), op_expr);
    Ok((path, filter))
}

/// Split a reference like `data->a->>b` into its column and JSON path.
///
/// A name with no arrows is returned unchanged, so this is safe to apply to
/// every field reference. A trailing fragment that does not parse as a path is
/// left attached to the column name rather than silently dropped -- the column
/// then fails to resolve, which is the honest outcome.
fn split_json_path(reference: &str) -> Field {
    let Some(arrow) = reference.find("->") else {
        return Field::simple(reference);
    };

    let (column, rest) = reference.split_at(arrow);
    match parse_json_path(rest) {
        Ok(("", json_path)) if !json_path.is_empty() => Field::with_json_path(column, json_path),
        _ => Field::simple(reference),
    }
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

    let path: Vec<String> = parts[..parts.len() - 1]
        .iter()
        .map(|s| s.to_string())
        .collect();
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

/// Map an operator name to its comparison operator, if it is one.
///
/// These are exactly the operators that accept an `any`/`all` quantifier.
fn quant_operator(name: &str) -> Option<QuantOperator> {
    Some(match name {
        "eq" => QuantOperator::Equal,
        "gt" => QuantOperator::GreaterThan,
        "gte" => QuantOperator::GreaterThanEqual,
        "lt" => QuantOperator::LessThan,
        "lte" => QuantOperator::LessThanEqual,
        "like" => QuantOperator::Like,
        "ilike" => QuantOperator::ILike,
        "match" => QuantOperator::Match,
        "imatch" => QuantOperator::IMatch,
        _ => return None,
    })
}

/// Parse the parenthesised modifier of a quantified comparison.
fn parse_quantifier(modifier: &str) -> Option<OpQuantifier> {
    match modifier {
        "any" => Some(OpQuantifier::Any),
        "all" => Some(OpQuantifier::All),
        _ => None,
    }
}

/// Parse an operation: `eq.value`, `in.(a,b,c)`, `is.null`, etc.
fn parse_operation(value: &str) -> Result<Operation> {
    // A quantified comparison spells the quantifier in parentheses after the
    // operator name: `col=like(any).{foo,bar}`. Full-text search borrows the
    // same shape for its language, so the operator name decides the reading.
    if let Some((name, rest)) = value.split_once('(') {
        if let Some((modifier, operand)) = rest.split_once(").") {
            if let (Some(op), Some(quantifier)) = (quant_operator(name), parse_quantifier(modifier))
            {
                return Ok(Operation::Quant {
                    op,
                    quantifier: Some(quantifier),
                    value: operand.to_string(),
                });
            }
        }
    }

    // Comparison operators. Every one of these accepts a quantifier, so they
    // share the lookup above rather than repeating the name-to-operator map.
    if let Some((name, rest)) = value.split_once('.') {
        if let Some(op) = quant_operator(name) {
            return Ok(Operation::Quant {
                op,
                quantifier: None,
                value: rest.to_string(),
            });
        }
    }

    if let Some(rest) = value.strip_prefix("neq.") {
        return Ok(Operation::Simple {
            op: SimpleOperator::NotEqual,
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
    let inner = value
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| Error::InvalidQueryParam(format!("in.{}", value)))?;

    let mut values = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = inner.chars();

    while let Some(c) = chars.next() {
        match c {
            // A value may be double-quoted so that it can contain a comma.
            // Inside the quotes a backslash escapes the next character, which
            // is the only way to write a literal `"` or `\`.
            '"' => quoted = !quoted,
            '\\' if quoted => {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            }
            ',' if !quoted => {
                values.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    values.push(current);

    Ok(values)
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
    let field = split_json_path(field_name);
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
        field,
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
        map(digit1, |s: &str| JsonOperand::Idx(s.parse().unwrap_or(0))),
        map(parse_identifier, |s| JsonOperand::Key(s.to_string())),
    ))(input)?;
    Ok((input, JsonOperation::Arrow(operand)))
}

fn parse_double_arrow(input: &str) -> IResult<&str, JsonOperation> {
    let (input, _) = tag("->>")(input)?;
    let (input, operand) = alt((
        map(digit1, |s: &str| JsonOperand::Idx(s.parse().unwrap_or(0))),
        map(parse_identifier, |s| JsonOperand::Key(s.to_string())),
    ))(input)?;
    Ok((input, JsonOperation::DoubleArrow(operand)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_filter() {
        let params = parse_query_params("name=eq.John", false).unwrap();
        assert_eq!(params.filters_root.len(), 1);
        assert_eq!(params.filters_root[0].field.name, "name");
    }

    #[test]
    fn test_parse_negated_filter() {
        let params = parse_query_params("status=not.eq.active", false).unwrap();
        assert!(params.filters_root[0].op_expr.negated);
    }

    #[test]
    fn test_parse_in_filter() {
        let params = parse_query_params("id=in.(1,2,3)", false).unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::In(values) => {
                assert_eq!(values, &vec!["1", "2", "3"]);
            }
            _ => panic!("Expected In operation"),
        }
    }

    #[test]
    fn test_parse_spread_relation_keeps_its_columns() {
        let items = parse_select("title,...directors(first_name,last_name)").unwrap();
        assert_eq!(items.len(), 2);
        match &items[1] {
            SelectItem::SpreadRelation {
                relation, select, ..
            } => {
                assert_eq!(relation, "directors");
                let names: Vec<_> = select
                    .iter()
                    .map(|item| match item {
                        SelectItem::Field { field, .. } => field.name.clone(),
                        other => panic!("expected a field, got {:?}", other),
                    })
                    .collect();
                assert_eq!(names, vec!["first_name", "last_name"]);
            }
            other => panic!("expected a spread relation, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_spread_relation_does_not_swallow_later_items() {
        // The column list used to be left unconsumed, which silently truncated
        // everything after it.
        let items = parse_select("...directors(name),year").unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], SelectItem::SpreadRelation { .. }));
        match &items[1] {
            SelectItem::Field { field, .. } => assert_eq!(field.name, "year"),
            other => panic!("expected a field, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_nested_spread_relation() {
        let items = parse_select("id,films(title,...directors(name))").unwrap();
        match &items[1] {
            SelectItem::Relation { select, .. } => {
                assert_eq!(select.len(), 2);
                assert!(matches!(select[1], SelectItem::SpreadRelation { .. }));
            }
            other => panic!("expected a relation, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_in_filter_with_quoted_commas() {
        let params = parse_query_params(r#"name=in.("hi,there","yes,you")"#, false).unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::In(values) => {
                assert_eq!(values, &vec!["hi,there".to_string(), "yes,you".to_string()]);
            }
            other => panic!("expected an In operation, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_in_filter_with_escapes() {
        let params = parse_query_params(r#"name=in.("a\"b","c\\d")"#, false).unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::In(values) => {
                assert_eq!(values, &vec![r#"a"b"#.to_string(), r#"c\d"#.to_string()]);
            }
            other => panic!("expected an In operation, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_quantified_filter() {
        for (query, expected) in [
            ("id=eq(any).{1,2,3}", OpQuantifier::Any),
            ("id=eq(all).{1,2,3}", OpQuantifier::All),
        ] {
            let params = parse_query_params(query, false).unwrap();
            match &params.filters_root[0].op_expr.operation {
                Operation::Quant {
                    op,
                    quantifier,
                    value,
                } => {
                    assert_eq!(op, &QuantOperator::Equal);
                    assert_eq!(quantifier.as_ref(), Some(&expected));
                    assert_eq!(value, "{1,2,3}");
                }
                other => panic!("expected a quantified operation, got {:?}", other),
            }
        }
    }

    #[test]
    fn test_parse_quantified_like_keeps_array_literal() {
        let params = parse_query_params("name=like(any).{foo*,bar*}", false).unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::Quant {
                op,
                quantifier,
                value,
            } => {
                assert_eq!(op, &QuantOperator::Like);
                assert_eq!(quantifier.as_ref(), Some(&OpQuantifier::Any));
                // The `*`-to-`%` mapping belongs to SQL generation, so the
                // parsed operand is still the literal the client sent.
                assert_eq!(value, "{foo*,bar*}");
            }
            other => panic!("expected a quantified operation, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_negated_quantified_filter() {
        let params = parse_query_params("id=not.eq(any).{1,2}", false).unwrap();
        assert!(params.filters_root[0].op_expr.negated);
        assert!(matches!(
            params.filters_root[0].op_expr.operation,
            Operation::Quant {
                quantifier: Some(OpQuantifier::Any),
                ..
            }
        ));
    }

    #[test]
    fn test_quantifier_form_does_not_capture_fts_language() {
        // `fts(english).x` has the same shape but is not a quantified
        // comparison; the operator name is what tells the two apart.
        let params = parse_query_params("body=fts(english).cat", false).unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::Fts { language, .. } => {
                assert_eq!(language.as_deref(), Some("english"));
            }
            other => panic!("expected an fts operation, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_is_null() {
        let params = parse_query_params("deleted_at=is.null", false).unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::Is(IsValue::Null) => {}
            _ => panic!("Expected Is Null"),
        }
    }

    #[test]
    fn test_parse_order() {
        let params = parse_query_params("order=name.asc,age.desc.nullslast", false).unwrap();
        assert_eq!(params.order.len(), 1);
        let (_, terms) = &params.order[0];
        assert_eq!(terms.len(), 2);
    }

    #[test]
    fn test_parse_limit_offset() {
        let params = parse_query_params("limit=10&offset=20", false).unwrap();
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
        let params = parse_query_params("content=fts(english).search+term", false).unwrap();
        match &params.filters_root[0].op_expr.operation {
            Operation::Fts {
                op,
                language,
                value,
            } => {
                assert_eq!(*op, FtsOperator::Fts);
                assert_eq!(language.as_deref(), Some("english"));
                assert_eq!(value, "search+term");
            }
            _ => panic!("Expected FTS operation"),
        }
    }
}

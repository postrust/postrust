//! Query parameter parsing using nom.
//!
//! Parses URL query strings into structured filter, select, order, and range data.
//! Mirrors PostgREST's QueryParams.hs parsing logic.

use super::types::*;
use crate::error::{Error, Result};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::char,
    combinator::{opt, value},
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

        // Modifiers come before filters, since both are dotted: `clients.order`
        // orders an embedded resource while `clients.name` filters one.
        if let Some((path, modifier)) = parse_modifier_key(key) {
            match modifier {
                Modifier::Order => {
                    let (_, terms) = parse_order_param(&decoded_value)?;
                    params.order.push((path, terms));
                }
                Modifier::Limit => {
                    let limit: i64 = decoded_value
                        .parse()
                        .map_err(|_| Error::InvalidQueryParam("limit".into()))?;
                    // Caught here rather than left to `LIMIT -1`, which
                    // PostgreSQL rejects with a message about SQL syntax for
                    // what is a plainly out-of-range request.
                    if limit < 0 {
                        return Err(Error::InvalidRange(
                            "Limit should be greater than or equal to zero.".into(),
                        ));
                    }
                    params.ranges.entry(path.join(".")).or_default().limit = Some(limit);
                }
                Modifier::Offset => {
                    let offset: i64 = decoded_value
                        .parse()
                        .map_err(|_| Error::InvalidQueryParam("offset".into()))?;
                    params.ranges.entry(path.join(".")).or_default().offset = offset;
                }
                Modifier::Logic(op, negated) => {
                    params
                        .logic
                        .push((path, parse_logic_param(op, negated, &decoded_value)?));
                }
            }
            continue;
        }

        match key {
            "select" => {
                params.select = parse_select(&decoded_value)?;
            }
            "columns" => {
                params.columns = Some(parse_columns(&decoded_value)?.into_iter().collect());
            }
            "on_conflict" => {
                params.on_conflict = Some(
                    decoded_value
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .collect(),
                );
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

/// Parse the `columns` parameter value.
///
/// A list of field names and nothing else, so the only thing that can go wrong
/// is a name that is not there -- `?columns=` names none, and `?columns=a,,b`
/// leaves a gap in the middle. Both were read as a field whose name is the
/// empty string, which got as far as the schema cache and came back as
/// "Could not find the '' column", describing a lookup the client never asked
/// for rather than the parameter it actually wrote.
fn parse_columns(input: &str) -> Result<Vec<String>> {
    const EXPECTED: &str = "expecting field name (* or [a..z0..9_$])";

    let mut columns = Vec::new();
    let mut at = 0usize;

    for (index, field) in input.split(',').enumerate() {
        if index > 0 {
            // The comma that separated this field from the last one.
            at += 1;
        }
        let trimmed = field.trim();
        if trimmed.is_empty() {
            return Err(Error::UnparsableQuery {
                kind: "columns parameter",
                text: input.to_string(),
                // Counting from one, at the character that should have begun
                // the name -- the end of the input where there is none left.
                column: at + 1,
                expected: match at >= input.len() {
                    true => format!("unexpected end of input {}", EXPECTED),
                    false => format!(
                        "unexpected {:?} {}",
                        input[at..].chars().next().unwrap_or_default(),
                        EXPECTED
                    ),
                },
            });
        }
        columns.push(trimmed.to_string());
        at += field.len();
    }

    Ok(columns)
}

/// Parse the `select` parameter value.
pub fn parse_select(input: &str) -> Result<Vec<SelectItem>> {
    if input.is_empty() {
        return Ok(vec![]);
    }

    // Anything left over is a parse failure, not something to ignore. Left
    // ignored, one unparsable item silently discarded every item after it --
    // `select=id, name, billing(address)` returned the id alone.
    match parse_select_items(input) {
        Ok(("", items)) => Ok(items),
        _ => Err(Error::InvalidQueryParam(format!("select={}", input))),
    }
}

fn parse_select_items(input: &str) -> IResult<&str, Vec<SelectItem>> {
    separated_list0(char(','), parse_select_item)(input)
}

fn parse_select_item(input: &str) -> IResult<&str, SelectItem> {
    // A space after a comma is ordinary in a hand-written URL, and PostgREST
    // accepts it.
    let (input, _) = nom::character::complete::space0(input)?;
    alt((
        parse_star,
        // Before relations: `count()` is spelled exactly like an embed of a
        // relation named `count` with an empty selection.
        parse_bare_aggregate,
        parse_spread_relation,
        parse_relation_select,
        parse_field_select,
    ))(input)
}

/// Parse `*`, which names every column.
///
/// It is a select item like any other, so it composes: `*,clients(id)` asks
/// for every column of this table and an embed besides, and `clients(*)` asks
/// for every column of the embedded one.
fn parse_star(input: &str) -> IResult<&str, SelectItem> {
    let (input, _) = char('*')(input)?;
    Ok((input, SelectItem::field("*")))
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
    let (input, relation) = parse_field_name(input)?;
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
            relation,
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
    let (input, name) = parse_field_name(input)?;
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
            relation: name,
            alias,
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

    let (input, name) = parse_field_name(input)?;
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
            field: Field { name, json_path },
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
fn parse_alias_prefix(input: &str) -> IResult<&str, String> {
    let (rest, name) = parse_field_name(input)?;
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
    let (path, field) = parse_filter_key(key)?;

    // Parse the value for operator and operand
    let op_expr = parse_filter_value(value)?;

    Ok((path, Filter::new(field, op_expr)))
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

/// Parse a filter key into the embedded path and the field it names.
///
/// The key is a run of field names separated by `.`, optionally followed by a
/// JSON path: `clients.id`, `data->>a`, `"a.dotted.column"`. Splitting on `.`
/// alone would cut a quoted name in half, and would take the dot of
/// `id.something` for a path separator, so the names are parsed rather than
/// split.
///
/// A key the grammar cannot account for falls back to the plain split, which
/// leaves whatever it produced to fail at column resolution -- an honest
/// outcome, and the one this had before.
fn parse_filter_key(key: &str) -> Result<(EmbedPath, Field)> {
    if let Some(parsed) = parse_field_reference(key) {
        return Ok(parsed);
    }

    let mut parts: Vec<&str> = key.split('.').collect();
    let field = parts
        .pop()
        .ok_or_else(|| Error::InvalidQueryParam(key.into()))?;
    let path = parts.into_iter().map(String::from).collect();
    Ok((path, split_json_path(field)))
}

/// A dotted path of field names followed by an optional JSON path.
///
/// `None` when the whole key was not consumed, which is the caller's signal
/// that this grammar does not describe it.
fn parse_field_reference(key: &str) -> Option<(EmbedPath, Field)> {
    let mut names = Vec::new();
    let mut rest = key;

    loop {
        let (remainder, name) = parse_field_name(rest).ok()?;
        names.push(name);
        match remainder.strip_prefix('.') {
            Some(after) => rest = after,
            None => {
                rest = remainder;
                break;
            }
        }
    }

    let (rest, json_path) = parse_json_path(rest).ok()?;
    if !rest.is_empty() {
        return None;
    }

    let field = names.pop()?;
    Some((names, Field::with_json_path(field, json_path)))
}

/// Whether a query parameter reads as a filter rather than a function argument.
///
/// On `/rpc/f?id=5&id=gt.2` the same name is both: `5` is the argument the
/// function takes and `gt.2` filters what it returned. What tells them apart
/// is the value -- an operator prefix means a filter -- so the classification
/// has to look at the value rather than the name.
pub fn value_is_filter(value: &str) -> bool {
    parse_filter_value(value).is_ok()
}

/// Parse filter value: `operator.value` or `not.operator.value`
fn parse_filter_value(value: &str) -> Result<OpExpr> {
    let whole = value;
    let (value, negated) = if let Some(rest) = value.strip_prefix("not.") {
        (rest, true)
    } else {
        (value, false)
    };

    let operation = parse_operation(value).map_err(|error| match error {
        // Nothing in the value named an operator, so reading stopped at its
        // first character -- and what a filter may begin with is a short,
        // published list. Any other failure got further in and has more to say
        // about where.
        Error::InvalidQueryParam(_) => Error::UnparsableQuery {
            kind: "filter",
            text: whole.to_string(),
            column: 1,
            expected: format!(
                "unexpected \"{}\" expecting \"not\" or operator (eq, gt, ...)",
                whole.chars().next().unwrap_or_default()
            ),
        },
        other => other,
    })?;
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
        // These are the only case-insensitive operands in the grammar, and
        // `not_null` has no operator of its own -- `is.not_null` is the only
        // spelling of `IS NOT NULL`.
        let is_val = match rest.to_ascii_lowercase().as_str() {
            "null" => IsValue::Null,
            "not_null" => IsValue::NotNull,
            "true" => IsValue::True,
            "false" => IsValue::False,
            "unknown" => IsValue::Unknown,
            _ => return Err(unreadable_is_value(rest)),
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

/// The four modifiers an order term accepts, and where reading one stopped.
///
/// Reading gets as far as the word still could be one of them and reports the
/// character that ruled the last one out, so `nullslasttt` fails on the tenth
/// character rather than on the word.
fn unreadable_order(term: &str, at: usize, part: &str) -> Error {
    const MODIFIERS: [&str; 4] = ["asc", "desc", "nullsfirst", "nullslast"];

    let read = MODIFIERS
        .iter()
        .map(|modifier| {
            part.chars()
                .zip(modifier.chars())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .max()
        .unwrap_or(0);

    Error::UnparsableQuery {
        kind: "order",
        text: term.to_string(),
        column: at + read + 1,
        expected: format!(
            "unexpected {:?} expecting \",\" or end of input",
            part.chars().nth(read).unwrap_or_default()
        ),
    }
}

/// The five things `is.` accepts, and where reading one stopped.
///
/// Reading gets as far as the operand still could be one of them and reports
/// the character that ruled the last one out -- `is.nil` fails on the `i`,
/// having read `n` as the start of `null`, while `is.ok` fails on the `o`.
/// That is where a client has to look, and it is the difference between a
/// message about a typo and one about the whole request.
fn unreadable_is_value(rest: &str) -> Error {
    const VALUES: [&str; 5] = ["null", "not_null", "true", "false", "unknown"];

    let lowered = rest.to_ascii_lowercase();
    let read = VALUES
        .iter()
        .map(|value| {
            lowered
                .chars()
                .zip(value.chars())
                .take_while(|(a, b)| a == b)
                .count()
        })
        .max()
        .unwrap_or(0);

    Error::UnparsableQuery {
        kind: "filter",
        text: format!("is.{}", rest),
        // Counting from one, past the `is.` that was read before this.
        column: 4 + read,
        expected: format!(
            "unexpected \"{}\" expecting isVal: ({})",
            rest.chars().nth(read).unwrap_or_default(),
            VALUES.join(", ")
        ),
    }
}

/// Parse IN list: `(a,b,c)` -> vec!["a", "b", "c"]
fn parse_in_list(value: &str) -> Result<Vec<String>> {
    // Whitespace immediately inside the parentheses is not part of the first
    // or last element: `in.(    )` is the empty list, while `in.( ,3,4)` has
    // an empty first element and is a type error the database reports.
    let inner = value
        .trim()
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .map(|inner| {
            inner
                .trim_start_matches([' ', '\t'])
                .trim_end_matches([' ', '\t'])
        })
        .ok_or_else(|| Error::InvalidQueryParam(format!("in.{}", value)))?;

    // `in.()` names no values at all, which is not the same as naming one
    // empty value -- and is what `?id=in.()` returning nothing means.
    if inner.is_empty() {
        return Ok(Vec::new());
    }

    let mut values = Vec::new();
    let mut rest = inner;

    loop {
        // Quoting is what lets a value contain the comma that would otherwise
        // end it, so it counts only when it spans the whole element: in
        // `Double"Quote"McGraw"` the quotes are part of the name, and taking
        // them for syntax would silently change what was asked for.
        let element = match quoted_element(rest) {
            Some((unquoted, remainder)) => {
                rest = remainder;
                unquoted
            }
            None => {
                let end = rest.find(',').unwrap_or(rest.len());
                let (element, remainder) = rest.split_at(end);
                rest = remainder;
                element.to_string()
            }
        };
        values.push(element);

        match rest.strip_prefix(',') {
            Some(remainder) => rest = remainder,
            None => break,
        }
    }

    Ok(values)
}

/// A complete double-quoted element at the head of `input`.
///
/// `None` unless the closing quote ends the element -- what follows it has to
/// be the comma that separates elements, or nothing at all.
fn quoted_element(input: &str) -> Option<(String, &str)> {
    let body = input.strip_prefix('"')?;

    let mut value = String::new();
    let mut chars = body.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => {
                let (_, escaped) = chars.next()?;
                value.push(escaped);
            }
            '"' => {
                let rest = &body[index + 1..];
                return match rest.is_empty() || rest.starts_with(',') {
                    true => Some((value, rest)),
                    false => None,
                };
            }
            other => value.push(other),
        }
    }

    None
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

    // Where each modifier starts within the term, so a word that is not one
    // can be reported at the character that made it not one.
    let mut at = field_name.len() + 1;
    for part in &parts[1..] {
        match *part {
            "asc" => direction = Some(OrderDirection::Asc),
            "desc" => direction = Some(OrderDirection::Desc),
            "nullsfirst" => nulls = Some(OrderNulls::First),
            "nullslast" => nulls = Some(OrderNulls::Last),
            // Not one of the four. Ignoring it answered a request nobody made:
            // `order=id.asc.nullslasttt` was read as `id.asc` and the typo
            // silently changed nothing, so a client asking for nulls last got
            // whatever the table happened to give it.
            other => return Err(unreadable_order(value, at, other)),
        }
        at += part.len() + 1;
    }

    // `clients(name)` orders by a column of an embedded resource rather than
    // by one of this table's own.
    if let Some((relation, rest)) = field_name.split_once('(') {
        if let Some(column) = rest.strip_suffix(')') {
            return Ok(OrderTerm::Relation {
                relation: relation.to_string(),
                field: split_json_path(column),
                direction,
                nulls,
            });
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
fn parse_logic_param(op: LogicOperator, negated: bool, value: &str) -> Result<LogicTree> {
    // Whitespace around the group and around each member is insignificant, as
    // it is in PostgREST: `and=( a.eq.1 , b.eq.2 )` is the same request as
    // `and=(a.eq.1,b.eq.2)`.
    // The group ends at the parenthesis that closes it, not at the last one in
    // the string: `and=(a.eq.1,b.eq.2))` is a well-formed group with a stray
    // character after it, and PostgREST reads it as one.
    let trimmed = value.trim();
    let inner = balanced_group(trimmed).ok_or_else(|| Error::InvalidQueryParam(value.into()))?;

    // Where a member is empty there is nothing to hand the filter parser, and
    // "Invalid request" says only that something in a URL the client wrote is
    // wrong. PostgREST names the character and what would have been accepted
    // there, counting from the start of the tree as it spells it -- `or=(...)`
    // is read as `or(...)`, so the `=` is not a column and the operator's own
    // name is.
    let members = split_top_level(inner);
    for member in &members {
        if !member.trim().is_empty() {
            continue;
        }
        let at = offset_within(trimmed, member);
        return Err(Error::UnparsableQuery {
            kind: "logic tree",
            text: trimmed.to_string(),
            column: negated as usize * "not.".len()
                + match op {
                    LogicOperator::And => 3,
                    LogicOperator::Or => 2,
                }
                + at
                + 1,
            expected: format!(
                "unexpected \"{}\" expecting field name (* or [a..z0..9_$]), \
                 negation operator (not) or logic operator (and, or)",
                trimmed[at..].chars().next().unwrap_or_default()
            ),
        });
    }

    Ok(LogicTree::Expr {
        negated,
        op,
        children: members
            .into_iter()
            .map(parse_logic_child)
            .collect::<Result<Vec<_>>>()?,
    })
}

/// Where `part` begins inside `whole`, which it is a subslice of.
///
/// The split loses the positions and the message needs them; taking them from
/// the pointers is exact where searching for the text would find the wrong
/// occurrence of a repeated member.
fn offset_within(whole: &str, part: &str) -> usize {
    (part.as_ptr() as usize).saturating_sub(whole.as_ptr() as usize)
}

/// The contents of a parenthesised group at the head of `input`.
///
/// Anything after the closing parenthesis is not part of the group and is
/// discarded, which is what a parser that does not demand end-of-input does.
fn balanced_group(input: &str) -> Option<&str> {
    let body = input.strip_prefix('(')?;

    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;

    for (idx, ch) in body.char_indices() {
        if quoted {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '(' => depth += 1,
            ')' if depth == 0 => return Some(&body[..idx]),
            ')' => depth -= 1,
            _ => {}
        }
    }

    None
}

/// Split a comma-separated list on its top-level commas only.
///
/// A logic list holds whole conditions, and a condition may contain commas of
/// its own -- `id.in.(1,2)`, `arr.cs.{1,2}`, a nested `and(...)`, or a quoted
/// value. Splitting on every comma tears those apart.
fn split_top_level(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut start = 0usize;

    for (idx, ch) in input.char_indices() {
        if quoted {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&input[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

/// Parse one member of a logic list: either a nested group or a condition.
fn parse_logic_child(item: &str) -> Result<LogicTree> {
    let item = item.trim();

    // `not.and(...)` negates the group. A leading `not.` on anything else
    // belongs to the condition's operator (`arr.not.cs.{1,2}`), so it is left
    // in place for the filter parser to deal with.
    let (body, negated) = match item.strip_prefix("not.") {
        Some(rest) if rest.starts_with("and(") || rest.starts_with("or(") => (rest, true),
        _ => (item, false),
    };

    for (name, op) in [("and", LogicOperator::And), ("or", LogicOperator::Or)] {
        if let Some(rest) = body.strip_prefix(name) {
            let rest = rest.trim_start();
            if rest.starts_with('(') && rest.ends_with(')') {
                return parse_logic_param(op, negated, rest);
            }
        }
    }

    let (key, val) = body
        .split_once('.')
        .ok_or_else(|| Error::InvalidQueryParam(item.into()))?;
    let (_, filter) = parse_filter_param(key, &unquote_logic_operand(val))?;
    Ok(LogicTree::Stmt(filter))
}

/// Strip the quotes from a logic-tree operand.
///
/// Inside `or=(...)` a value may be quoted so that it can contain the comma
/// that would otherwise end it -- `name.eq."(grandchild,entity,4)"` -- and the
/// quotes are then syntax rather than part of the value. Outside a logic tree
/// there is nothing to protect the value from, so a quote there is a
/// character like any other and is left alone.
fn unquote_logic_operand(value: &str) -> String {
    let Some(open) = value.find('"') else {
        return value.to_string();
    };
    // The quote has to open the operand rather than sit inside one: `id.in.(
    // "a","b")` is a list, which does its own unquoting element by element.
    if open != 0 && !value[..open].ends_with('.') {
        return value.to_string();
    }
    if !value.ends_with('"') || value.len() < open + 2 {
        return value.to_string();
    }

    let mut unquoted = String::from(&value[..open]);
    let mut chars = value[open + 1..value.len() - 1].chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(escaped) = chars.next() {
                    unquoted.push(escaped);
                }
            }
            other => unquoted.push(other),
        }
    }
    unquoted
}

/// A query parameter that modifies a resource rather than filtering it.
enum Modifier {
    Order,
    Limit,
    Offset,
    Logic(LogicOperator, bool),
}

/// Recognise a modifier key and the embedded resource it addresses.
///
/// Every modifier may be aimed at an embedded resource by prefixing it with
/// the path to that resource: `clients.order=name` orders the embedded
/// clients, `clients.limit=1` pages them, `clients.or=(...)` filters them. The
/// bare forms are the same thing with an empty path.
fn parse_modifier_key(key: &str) -> Option<(EmbedPath, Modifier)> {
    let mut parts: Vec<&str> = key.split('.').collect();
    let last = parts.pop()?;

    let modifier = match last {
        "order" => Modifier::Order,
        "limit" => Modifier::Limit,
        "offset" => Modifier::Offset,
        "and" | "or" => {
            let op = if last == "and" {
                LogicOperator::And
            } else {
                LogicOperator::Or
            };
            // `not.or=(...)` negates the whole group.
            let negated = parts.last() == Some(&"not");
            if negated {
                parts.pop();
            }
            Modifier::Logic(op, negated)
        }
        _ => return None,
    };

    Some((parts.into_iter().map(String::from).collect(), modifier))
}

// ============================================================================
// Helper Parsers
// ============================================================================

/// The characters a bare identifier is made of.
///
/// PostgREST's set exactly: letters, digits, `_`, `$` and the space, with the
/// result trimmed. A space is allowed because a column may genuinely have one
/// -- `?select=Just A Server Model` -- and trimming is what keeps
/// `?select=id, name` from asking for a column called `name ` .
fn parse_identifier(input: &str) -> IResult<&str, &str> {
    let (rest, matched) =
        take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == '$' || c == ' ')(input)?;
    Ok((rest, matched.trim()))
}

/// Parse a field name: a quoted name, or dash-joined identifiers.
///
/// Quoting is how a client names a column the bare grammar cannot spell --
/// `"a.dotted.column"`, `"(inside,parens)"` -- and a backslash escapes the
/// next character inside it.
///
/// Unquoted, a `-` joins identifiers into one name, so `field-with_sep` is a
/// single column. A `-` that starts `->` is not a join: that is a JSON path,
/// and it belongs to whatever comes after the name.
fn parse_field_name(input: &str) -> IResult<&str, String> {
    if let Ok(quoted) = parse_quoted_name(input) {
        return Ok(quoted);
    }

    let (mut rest, first) = parse_identifier(input)?;
    let mut name = first.to_string();

    while let Some(after_dash) = rest.strip_prefix('-') {
        if after_dash.starts_with('>') {
            break;
        }
        let Ok((next, segment)) = parse_identifier(after_dash) else {
            break;
        };
        name.push('-');
        name.push_str(segment);
        rest = next;
    }

    Ok((rest, name))
}

/// Parse a double-quoted name, honouring backslash escapes.
fn parse_quoted_name(input: &str) -> IResult<&str, String> {
    let (rest, _) = char('"')(input)?;

    let mut name = String::new();
    let mut chars = rest.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '\\' => {
                if let Some((_, escaped)) = chars.next() {
                    name.push(escaped);
                }
            }
            '"' => return Ok((&rest[index + 1..], name)),
            other => name.push(other),
        }
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input,
        nom::error::ErrorKind::Verify,
    )))
}

fn parse_json_path(input: &str) -> IResult<&str, JsonPath> {
    // `->>` first: it starts with `->`, so trying the shorter one first would
    // match it and leave the second `>` at the head of the key.
    many0(alt((parse_double_arrow, parse_arrow)))(input)
}

/// Parse one step of a JSON path.
///
/// A JSON object key can be very nearly anything, and PostgREST's grammar says
/// so: everything up to the next thing the select grammar itself reserves is
/// the key. `data->>!@#$%^&*_e` and `data->23-xy-45` are ordinary keys, not
/// syntax errors, and reading a key as alphanumeric-only silently truncated
/// the path and returned the whole column instead.
///
/// A key made entirely of digits is an array index, which is why this decides
/// between the two only once the whole key has been read: `data->0xy1` steps
/// into the key `0xy1`, not into element 0 of an array.
fn parse_json_operand(input: &str) -> IResult<&str, JsonOperand> {
    let mut end = input.len();
    for (idx, ch) in input.char_indices() {
        // `,` ends the select item, `(` and `)` bound an embedding, `:` starts
        // a cast or an alias, and `->` starts the next step.
        if matches!(ch, ',' | '(' | ')' | ':') || input[idx..].starts_with("->") {
            end = idx;
            break;
        }
    }

    let (key, rest) = input.split_at(end);
    if key.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::TakeWhile1,
        )));
    }

    let operand = match key.parse::<i32>() {
        Ok(index) => JsonOperand::Idx(index),
        // A leading `-` begins an array index counted from the end, so what
        // follows it has to be a number. `data->>--34` was read as a key
        // literally named `--34` and answered 200 with nulls, where the
        // grammar has no such key in it and PostgREST says so.
        // A `Failure` rather than an `Error`: the arrow has already been read,
        // so this is not a step that some other rule might match instead.
        // Reported as a recoverable error, `alt` backtracked from `->>` to
        // `->` and read the rest as a key beginning with `>`, which turned a
        // malformed index into an ordinary lookup that answered 200.
        Err(_) if key.starts_with('-') => {
            return Err(nom::Err::Failure(nom::error::Error::new(
                &input[1..],
                nom::error::ErrorKind::Digit,
            )))
        }
        Err(_) => JsonOperand::Key(key.to_string()),
    };
    Ok((rest, operand))
}

fn parse_arrow(input: &str) -> IResult<&str, JsonOperation> {
    let (input, _) = tag("->")(input)?;
    let (input, operand) = parse_json_operand(input)?;
    Ok((input, JsonOperation::Arrow(operand)))
}

fn parse_double_arrow(input: &str) -> IResult<&str, JsonOperation> {
    let (input, _) = tag("->>")(input)?;
    let (input, operand) = parse_json_operand(input)?;
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
    fn test_json_path_double_arrow_wins_over_single() {
        let items = parse_select("settings->foo->>bar").unwrap();
        match &items[0] {
            SelectItem::Field { field, .. } => {
                assert_eq!(
                    field.json_path,
                    vec![
                        JsonOperation::Arrow(JsonOperand::Key("foo".into())),
                        JsonOperation::DoubleArrow(JsonOperand::Key("bar".into())),
                    ]
                );
            }
            other => panic!("expected a field, got {:?}", other),
        }
    }

    #[test]
    fn test_json_path_accepts_awkward_keys() {
        // PostgREST reserves only what its own grammar needs, so a key may
        // hold punctuation and hyphens.
        let items = parse_select("data->23-xy-45->>!@#$%^&*_e").unwrap();
        match &items[0] {
            SelectItem::Field { field, .. } => {
                assert_eq!(
                    field.json_path,
                    vec![
                        JsonOperation::Arrow(JsonOperand::Key("23-xy-45".into())),
                        JsonOperation::DoubleArrow(JsonOperand::Key("!@#$%^&*_e".into())),
                    ]
                );
            }
            other => panic!("expected a field, got {:?}", other),
        }
    }

    #[test]
    fn test_json_path_digits_are_an_index_but_only_when_whole() {
        let items = parse_select("data->0,other->0xy1").unwrap();
        let paths: Vec<_> = items
            .iter()
            .map(|i| match i {
                SelectItem::Field { field, .. } => field.json_path.clone(),
                other => panic!("expected a field, got {:?}", other),
            })
            .collect();
        assert_eq!(paths[0], vec![JsonOperation::Arrow(JsonOperand::Idx(0))]);
        assert_eq!(
            paths[1],
            vec![JsonOperation::Arrow(JsonOperand::Key("0xy1".into()))]
        );
    }

    #[test]
    fn test_json_path_stops_at_a_cast() {
        let items = parse_select("settings->>foo::json").unwrap();
        match &items[0] {
            SelectItem::Field { field, cast, .. } => {
                assert_eq!(
                    field.json_path,
                    vec![JsonOperation::DoubleArrow(JsonOperand::Key("foo".into()))]
                );
                assert_eq!(cast.as_deref(), Some("json"));
            }
            other => panic!("expected a field, got {:?}", other),
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
    /// `?columns=` names no field, and is reported as the parameter it is.
    #[test]
    fn an_empty_columns_parameter_is_a_parse_error() {
        let error = parse_columns("").unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"failed to parse columns parameter ()\" (line 1, column 1)"
        );
        match error {
            Error::UnparsableQuery { expected, .. } => assert_eq!(
                expected,
                "unexpected end of input expecting field name (* or [a..z0..9_$])"
            ),
            other => panic!("wrong variant: {:?}", other),
        }
    }

    /// A gap in the middle is reported where the gap is, not at the end.
    #[test]
    fn a_missing_field_in_columns_is_reported_where_it_is() {
        let error = parse_columns("a,,b").unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"failed to parse columns parameter (a,,b)\" (line 1, column 3)"
        );
    }

    #[test]
    fn columns_parses_a_plain_list() {
        assert_eq!(
            parse_columns("a, b ,c").unwrap(),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    /// A `-` inside a JSON path begins an index, so a non-number after it is
    /// not a key by another name.
    #[test]
    fn a_json_path_index_that_is_not_a_number_is_refused() {
        assert!(parse_select("data->>--34").is_err());
        assert!(parse_select("data->>-34").is_ok());
        assert!(parse_select("data->>34").is_ok());
        assert!(parse_select("data->>key").is_ok());
    }
    /// An empty member of a logic tree is reported the way PostgREST reports
    /// it, which counts from the tree as it spells it: `or=(...)` is read as
    /// `or(...)`, so the `=` is not a column and the operator's name is.
    #[test]
    fn an_empty_logic_tree_member_names_the_character_and_the_column() {
        let error = parse_logic_param(LogicOperator::Or, false, "()").unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"failed to parse logic tree (())\" (line 1, column 4)"
        );
        match &error {
            Error::UnparsableQuery { expected, .. } => assert_eq!(
                expected,
                "unexpected \")\" expecting field name (* or [a..z0..9_$]), \
                 negation operator (not) or logic operator (and, or)"
            ),
            other => panic!("wrong variant: {:?}", other),
        }

        // `and` is a longer name, so the same shape reports one column later.
        let error = parse_logic_param(LogicOperator::And, false, "()").unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"failed to parse logic tree (())\" (line 1, column 5)"
        );

        // A gap between two members is reported where the gap is.
        let error = parse_logic_param(LogicOperator::Or, false, "(,)").unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"failed to parse logic tree ((,))\" (line 1, column 4)"
        );
    }

}

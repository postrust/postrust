//! SQL queries for schema introspection.

use super::relationship::{
    Cardinality, MediaHandler, MediaHandlerMap, Relationship, RelationshipsMap,
};
use super::routine::{FuncVolatility, RetType, Routine, RoutineMap, RoutineParam};
use super::table::{Column, ColumnMap, Table, TablesMap};
use crate::api_request::QualifiedIdentifier;
use crate::error::{Error, Result};
use indexmap::IndexMap;
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};

/// Get PostgreSQL version.
pub async fn get_pg_version(pool: &PgPool) -> Result<i32> {
    let row = sqlx::query("SELECT current_setting('server_version_num')::int as version")
        .fetch_one(pool)
        .await
        .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    Ok(row.get("version"))
}

/// Load all tables and their columns.
pub async fn load_tables(pool: &PgPool, schemas: &[String]) -> Result<TablesMap> {
    let mut tables = HashMap::new();

    // Query tables/views from information_schema
    let rows = sqlx::query(
        r#"
        SELECT
            t.table_schema,
            t.table_name,
            t.table_type,
            pg_catalog.obj_description(
                (quote_ident(t.table_schema) || '.' || quote_ident(t.table_name))::regclass
            ) as description,
            COALESCE(
                (SELECT array_agg(a.attname ORDER BY array_position(i.indkey, a.attnum))
                FROM pg_index i
                JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
                WHERE i.indrelid = (quote_ident(t.table_schema) || '.' || quote_ident(t.table_name))::regclass
                  AND i.indisprimary),
                ARRAY[]::text[]
            ) as pk_cols,
            EXISTS (
                SELECT 1 FROM information_schema.table_privileges tp
                WHERE tp.table_schema = t.table_schema
                  AND tp.table_name = t.table_name
                  AND tp.privilege_type = 'INSERT'
            ) as insertable,
            EXISTS (
                SELECT 1 FROM information_schema.table_privileges tp
                WHERE tp.table_schema = t.table_schema
                  AND tp.table_name = t.table_name
                  AND tp.privilege_type = 'UPDATE'
            ) as updatable,
            EXISTS (
                SELECT 1 FROM information_schema.table_privileges tp
                WHERE tp.table_schema = t.table_schema
                  AND tp.table_name = t.table_name
                  AND tp.privilege_type = 'DELETE'
            ) as deletable
        FROM information_schema.tables t
        WHERE t.table_schema = ANY($1)
          AND t.table_type IN ('BASE TABLE', 'VIEW')
        ORDER BY t.table_schema, t.table_name
        "#,
    )
    .bind(schemas)
    .fetch_all(pool)
    .await
    .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    for row in rows {
        let schema: String = row.get("table_schema");
        let name: String = row.get("table_name");
        let qi = QualifiedIdentifier::new(&schema, &name);

        let table_type: String = row.get("table_type");
        let pk_cols: Vec<String> = row.get("pk_cols");

        let table = Table {
            schema: schema.clone(),
            name: name.clone(),
            description: row.get("description"),
            is_view: table_type == "VIEW",
            insertable: row.get("insertable"),
            updatable: row.get("updatable"),
            deletable: row.get("deletable"),
            pk_cols: pk_cols.clone(),
            columns: load_columns(pool, &schema, &name, &pk_cols).await?,
        };

        tables.insert(qi, table);
    }

    Ok(tables)
}

/// Load columns for a table.
async fn load_columns(
    pool: &PgPool,
    schema: &str,
    table: &str,
    pk_cols: &[String],
) -> Result<ColumnMap> {
    let mut columns = IndexMap::new();

    let rows = sqlx::query(
        r#"
        SELECT
            c.column_name,
            c.ordinal_position,
            c.is_nullable,
            c.data_type,
            c.udt_name,
            c.character_maximum_length,
            c.column_default,
            pg_catalog.col_description(
                (quote_ident(c.table_schema) || '.' || quote_ident(c.table_name))::regclass,
                c.ordinal_position
            ) as description,
            CASE WHEN e.enumtypid IS NOT NULL
                 THEN array_agg(e.enumlabel ORDER BY e.enumsortorder)
                 ELSE ARRAY[]::text[]
            END as enum_values
        FROM information_schema.columns c
        LEFT JOIN pg_type t ON t.typname = c.udt_name
        LEFT JOIN pg_enum e ON e.enumtypid = t.oid
        WHERE c.table_schema = $1 AND c.table_name = $2
        GROUP BY c.table_schema, c.table_name, c.column_name, c.ordinal_position, c.is_nullable,
                 c.data_type, c.udt_name, c.character_maximum_length,
                 c.column_default, t.oid, e.enumtypid
        ORDER BY c.ordinal_position
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    for row in rows {
        let name: String = row.get("column_name");
        let is_nullable: String = row.get("is_nullable");
        let data_type: String = row.get("data_type");
        let udt_name: String = row.get("udt_name");
        let max_len: Option<i32> = row.get("character_maximum_length");
        let enum_values: Vec<String> = row.get("enum_values");
        let position: i32 = row.get("ordinal_position");

        let column = Column {
            name: name.clone(),
            description: row.get("description"),
            nullable: is_nullable == "YES",
            data_type,
            nominal_type: udt_name,
            max_len,
            default: row.get("column_default"),
            enum_values,
            is_pk: pk_cols.contains(&name),
            position,
        };

        columns.insert(name, column);
    }

    Ok(columns)
}

/// Load foreign key relationships.
pub async fn load_relationships(pool: &PgPool, schemas: &[String]) -> Result<RelationshipsMap> {
    let mut relationships: RelationshipsMap = HashMap::new();

    let rows = sqlx::query(
        r#"
        SELECT
            c.conname as constraint_name,
            ns1.nspname as table_schema,
            t1.relname as table_name,
            ns2.nspname as foreign_table_schema,
            t2.relname as foreign_table_name,
            -- Each key's columns are read in their own subquery, by ordinal
            -- position. Joining pg_attribute twice in the outer query instead
            -- would multiply the two keys together: a two-column foreign key
            -- came back as four pairs, of which two paired a column with the
            -- wrong one on the other side.
            (SELECT array_agg(a.attname ORDER BY k.ord)
               FROM unnest(c.conkey) WITH ORDINALITY AS k(attnum, ord)
               JOIN pg_attribute a
                 ON a.attrelid = c.conrelid AND a.attnum = k.attnum) as columns,
            (SELECT array_agg(a.attname ORDER BY k.ord)
               FROM unnest(c.confkey) WITH ORDINALITY AS k(attnum, ord)
               JOIN pg_attribute a
                 ON a.attrelid = c.confrelid AND a.attnum = k.attnum) as foreign_columns,
            t1.relkind = 'v' as table_is_view,
            t2.relkind = 'v' as foreign_table_is_view,
            -- The key is unique when a unique index is *covered by* it, not
            -- when the index merely contains it. Asked the other way round,
            -- any key that is part of a wider unique index looked unique --
            -- so a foreign key on two of a three-column primary key became a
            -- one-to-one, and the far side embedded a single object where it
            -- should have embedded an array.
            EXISTS (
                SELECT 1 FROM pg_index i
                WHERE i.indrelid = c.conrelid
                  AND i.indisunique
                  AND c.conkey::int[] @> i.indkey::int[]
            ) as is_unique
        FROM pg_constraint c
        JOIN pg_class t1 ON t1.oid = c.conrelid
        JOIN pg_namespace ns1 ON ns1.oid = t1.relnamespace
        JOIN pg_class t2 ON t2.oid = c.confrelid
        JOIN pg_namespace ns2 ON ns2.oid = t2.relnamespace
        WHERE c.contype = 'f'
          AND ns1.nspname = ANY($1)
        "#,
    )
    .bind(schemas)
    .fetch_all(pool)
    .await
    .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    for row in rows {
        let table_schema: String = row.get("table_schema");
        let table_name: String = row.get("table_name");
        let foreign_schema: String = row.get("foreign_table_schema");
        let foreign_name: String = row.get("foreign_table_name");
        let constraint_name: String = row.get("constraint_name");
        let columns: Vec<String> = row.get("columns");
        let foreign_columns: Vec<String> = row.get("foreign_columns");
        let table_is_view: bool = row.get("table_is_view");
        let foreign_is_view: bool = row.get("foreign_table_is_view");
        let is_unique: bool = row.get("is_unique");

        let table_qi = QualifiedIdentifier::new(&table_schema, &table_name);
        let foreign_qi = QualifiedIdentifier::new(&foreign_schema, &foreign_name);

        let column_pairs: Vec<(String, String)> =
            columns.into_iter().zip(foreign_columns).collect();

        let is_self = table_qi == foreign_qi;

        // M2O relationship (this table has FK to foreign table)
        let cardinality = if is_unique {
            Cardinality::O2O {
                constraint: constraint_name.clone(),
                columns: column_pairs.clone(),
                is_parent: false,
            }
        } else {
            Cardinality::M2O {
                constraint: constraint_name.clone(),
                columns: column_pairs.clone(),
            }
        };

        let rel = Relationship::ForeignKey {
            table: table_qi.clone(),
            foreign_table: foreign_qi.clone(),
            is_self,
            cardinality,
            table_is_view,
            foreign_table_is_view: foreign_is_view,
            constraint_name: constraint_name.clone(),
        };

        relationships
            .entry((table_qi.clone(), table_schema.clone()))
            .or_default()
            .push(rel);

        // O2M relationship (foreign table has many of this table)
        let reverse_columns: Vec<(String, String)> = column_pairs
            .iter()
            .map(|(a, b)| (b.clone(), a.clone()))
            .collect();

        let reverse_cardinality = if is_unique {
            Cardinality::O2O {
                constraint: constraint_name.clone(),
                columns: reverse_columns.clone(),
                is_parent: true,
            }
        } else {
            // Reversed, like the O2O case above: column pairs are always
            // (local column, foreign column) relative to the table the
            // relationship is stored under. For this direction the local table
            // is the referenced one, so the pair must be swapped.
            Cardinality::O2M {
                constraint: constraint_name.clone(),
                columns: reverse_columns,
            }
        };

        let reverse_rel = Relationship::ForeignKey {
            table: foreign_qi.clone(),
            foreign_table: table_qi,
            is_self,
            cardinality: reverse_cardinality,
            table_is_view: foreign_is_view,
            foreign_table_is_view: table_is_view,
            constraint_name,
        };

        relationships
            .entry((foreign_qi, foreign_schema))
            .or_default()
            .push(reverse_rel);
    }

    load_computed_relationships(pool, schemas, &mut relationships).await?;

    Ok(relationships)
}

/// Add relationships that are computed by a function rather than declared by a
/// foreign key.
///
/// A function qualifies when it takes exactly one argument, that argument is
/// the composite type of an exposed table, and it returns rows of an exposed
/// table. `SETOF t` yields many rows per parent and a bare `t` yields one;
/// `ROWS 1` on a set-returning function declares it yields one as well, which
/// is how a to-one computed relationship is written.
///
/// The relationship is named after the function, not after the table it
/// returns, so that two functions returning the same table stay distinct.
async fn load_computed_relationships(
    pool: &PgPool,
    schemas: &[String],
    relationships: &mut RelationshipsMap,
) -> Result<()> {
    let rows = sqlx::query(
        r#"
        SELECT
            pn.nspname  AS function_schema,
            p.proname   AS function_name,
            tn.nspname  AS table_schema,
            t.relname   AS table_name,
            fn.nspname  AS foreign_table_schema,
            f.relname   AS foreign_table_name,
            (NOT p.proretset) OR p.prorows = 1 AS to_one
        FROM pg_proc p
        JOIN pg_namespace pn ON pn.oid = p.pronamespace
        -- the single argument is a table's composite type
        JOIN pg_type argt ON argt.oid = p.proargtypes[0]
        JOIN pg_class t   ON t.oid = argt.typrelid
        JOIN pg_namespace tn ON tn.oid = t.relnamespace
        -- and the return type is a table as well
        JOIN pg_type rett ON rett.oid = p.prorettype
        JOIN pg_class f   ON f.oid = rett.typrelid
        JOIN pg_namespace fn ON fn.oid = f.relnamespace
        WHERE p.pronargs = 1
          AND pn.nspname = ANY($1)
          AND tn.nspname = ANY($1)
          AND fn.nspname = ANY($1)
          -- relkind 'c' would be a standalone composite type, not a relation
          AND t.relkind = ANY (ARRAY['r','v','m','f','p'])
          AND f.relkind = ANY (ARRAY['r','v','m','f','p'])
        "#,
    )
    .bind(schemas)
    .fetch_all(pool)
    .await
    .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    for row in rows {
        let function = QualifiedIdentifier::new(
            row.get::<String, _>("function_schema"),
            row.get::<String, _>("function_name"),
        );
        let table_schema: String = row.get("table_schema");
        let table = QualifiedIdentifier::new(&table_schema, row.get::<String, _>("table_name"));
        let foreign_table = QualifiedIdentifier::new(
            row.get::<String, _>("foreign_table_schema"),
            row.get::<String, _>("foreign_table_name"),
        );
        let is_self = table == foreign_table;

        relationships
            .entry((table.clone(), table_schema))
            .or_default()
            .push(Relationship::Computed {
                function,
                table: table.clone(),
                foreign_table: foreign_table.clone(),
                table_alias: table,
                to_one: row.get("to_one"),
                is_self,
            });
    }

    Ok(())
}

/// Decode the `params` JSON built by [`load_routines`]'s query.
///
/// Anything malformed yields no parameters rather than an error: an argument
/// whose type we don't know is bound untyped, which is how the server behaved
/// before parameter types were loaded at all.
fn parse_routine_params(value: serde_json::Value) -> Vec<RoutineParam> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let param_type = item.get("type")?.as_str()?.to_string();
            Some(RoutineParam {
                name: item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string(),
                type_max_length: param_type.clone(),
                param_type,
                required: item
                    .get("required")
                    .and_then(|r| r.as_bool())
                    .unwrap_or(true),
                variadic: item
                    .get("variadic")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// Load stored functions.
pub async fn load_routines(pool: &PgPool, schemas: &[String]) -> Result<RoutineMap> {
    let mut routines: RoutineMap = HashMap::new();

    let rows = sqlx::query(
        r#"
        SELECT
            n.nspname as schema,
            p.proname as name,
            pg_catalog.obj_description(p.oid) as description,
            p.provolatile::text as volatility,
            p.provariadic <> 0 as has_variadic,
            p.prokind = 'p' as is_procedure,
            pg_get_function_identity_arguments(p.oid) as args,
            -- Input parameters, in declaration order. Built from the catalog
            -- rather than parsed out of the identity-arguments string, whose
            -- types can themselves contain commas (`numeric(10,2)`).
            -- `proargtypes` covers the IN parameters only and is 0-indexed;
            -- `proargnames` is 1-indexed and lists IN names first, so the two
            -- line up over 1..pronargs. A parameter is optional when it falls
            -- inside the trailing run that has defaults.
            COALESCE((
                SELECT json_agg(
                    json_build_object(
                        'name', COALESCE(p.proargnames[i], ''),
                        'type', pg_catalog.format_type(p.proargtypes[i - 1], NULL),
                        'required', i <= (p.pronargs - p.pronargdefaults),
                        'variadic', p.provariadic <> 0 AND i = p.pronargs
                    ) ORDER BY i
                )
                FROM generate_series(1, p.pronargs) AS i
            ), '[]'::json) as params,
            CASE
                WHEN p.proretset THEN 'SETOF ' || pg_catalog.format_type(p.prorettype, NULL)
                ELSE pg_catalog.format_type(p.prorettype, NULL)
            END as return_type,
            p.proretset as returns_set,
            (SELECT t.typtype::text FROM pg_catalog.pg_type t WHERE t.oid = p.prorettype)
                as ret_typtype
        FROM pg_proc p
        JOIN pg_namespace n ON n.oid = p.pronamespace
        WHERE n.nspname = ANY($1)
          AND p.prokind IN ('f', 'p')
        ORDER BY n.nspname, p.proname
        "#,
    )
    .bind(schemas)
    .fetch_all(pool)
    .await
    .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    for row in rows {
        let schema: String = row.get("schema");
        let name: String = row.get("name");
        let qi = QualifiedIdentifier::new(&schema, &name);

        let volatility: String = row.get("volatility");
        let return_type_str: String = row.get("return_type");
        let returns_set: bool = row.get("returns_set");

        let return_type = if return_type_str == "void" {
            RetType::Void
        } else if returns_set {
            RetType::SetOf(return_type_str.replace("SETOF ", ""))
        } else {
            RetType::Single(return_type_str)
        };

        // Composite ('c') and pseudo ('p', e.g. `record` from RETURNS TABLE)
        // return types expand to their own columns in `SELECT * FROM fn()`;
        // scalar returns yield a single column named after the function. Void
        // is pseudo too but renders as a function-named null column, so it
        // counts as non-composite.
        let ret_typtype: Option<String> = row.get("ret_typtype");
        let returns_composite = !matches!(return_type, RetType::Void)
            && matches!(ret_typtype.as_deref(), Some("c") | Some("p"));

        let routine = Routine {
            schema,
            name,
            description: row.get("description"),
            params: parse_routine_params(row.get("params")),
            return_type,
            returns_composite,
            volatility: FuncVolatility::from_char(volatility.chars().next().unwrap_or('v')),
            has_variadic: row.get("has_variadic"),
            isolation_level: None,
            settings: vec![],
            is_procedure: row.get("is_procedure"),
        };

        routines.entry(qi).or_default().push(routine);
    }

    Ok(routines)
}

/// Load valid timezone names.
pub async fn load_timezones(pool: &PgPool) -> Result<HashSet<String>> {
    let rows = sqlx::query("SELECT name FROM pg_timezone_names")
        .fetch_all(pool)
        .await
        .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    Ok(rows.iter().map(|r| r.get("name")).collect())
}

/// Load user-defined renderers for media types.
///
/// A renderer is an aggregate whose state type is a domain named after a media
/// type. The domain is how the name survives -- `application/geo+json` is not
/// a legal identifier otherwise -- and the `/` in it is what makes an
/// ordinary-looking domain recognisable as one.
///
/// The aggregate's argument says what it renders: a table's composite type
/// renders that table, and `anyelement` renders anything in the schema.
pub async fn load_media_handlers(pool: &PgPool, schemas: &[String]) -> Result<MediaHandlerMap> {
    let rows = sqlx::query(
        r#"
        SELECT
            n.nspname    AS agg_schema,
            p.proname    AS agg_name,
            t.typname    AS media_type,
            argn.nspname AS arg_schema,
            argc.relname AS arg_table
        FROM pg_aggregate a
        JOIN pg_proc p       ON p.oid = a.aggfnoid
        JOIN pg_namespace n  ON n.oid = p.pronamespace
        JOIN pg_type t       ON t.oid = a.aggtranstype
        LEFT JOIN pg_type argt      ON argt.oid = p.proargtypes[0]
        LEFT JOIN pg_class argc     ON argc.oid = argt.typrelid
        LEFT JOIN pg_namespace argn ON argn.oid = argc.relnamespace
        WHERE n.nspname = ANY($1)
          AND t.typtype = 'd'
          AND t.typname LIKE '%/%'
        "#,
    )
    .bind(schemas)
    .fetch_all(pool)
    .await
    .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?;

    let mut handlers: MediaHandlerMap = HashMap::new();
    for row in rows {
        let agg_schema: String = row.get("agg_schema");
        let media_type: String = row.get("media_type");
        let table = match (
            row.get::<Option<String>, _>("arg_schema"),
            row.get::<Option<String>, _>("arg_table"),
        ) {
            (Some(schema), Some(name)) => Some(QualifiedIdentifier::new(schema, name)),
            _ => None,
        };

        handlers
            .entry((agg_schema.clone(), media_type))
            .or_default()
            .push(MediaHandler {
                aggregate: QualifiedIdentifier::new(agg_schema, row.get::<String, _>("agg_name")),
                table,
            });
    }

    Ok(handlers)
}

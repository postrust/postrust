//! SQL queries for schema introspection.

use super::table::{Column, ColumnMap, Table, TablesMap};
use super::relationship::{Cardinality, Relationship, RelationshipsMap};
use super::routine::{FuncVolatility, RetType, Routine, RoutineMap};
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
            array_agg(a1.attname ORDER BY array_position(c.conkey, a1.attnum)) as columns,
            array_agg(a2.attname ORDER BY array_position(c.confkey, a2.attnum)) as foreign_columns,
            t1.relkind = 'v' as table_is_view,
            t2.relkind = 'v' as foreign_table_is_view,
            EXISTS (
                SELECT 1 FROM pg_index i
                WHERE i.indrelid = c.conrelid
                  AND i.indisunique
                  AND i.indkey::int[] @> c.conkey::int[]
            ) as is_unique
        FROM pg_constraint c
        JOIN pg_class t1 ON t1.oid = c.conrelid
        JOIN pg_namespace ns1 ON ns1.oid = t1.relnamespace
        JOIN pg_class t2 ON t2.oid = c.confrelid
        JOIN pg_namespace ns2 ON ns2.oid = t2.relnamespace
        JOIN pg_attribute a1 ON a1.attrelid = c.conrelid AND a1.attnum = ANY(c.conkey)
        JOIN pg_attribute a2 ON a2.attrelid = c.confrelid AND a2.attnum = ANY(c.confkey)
        WHERE c.contype = 'f'
          AND ns1.nspname = ANY($1)
        GROUP BY c.conname, ns1.nspname, t1.relname, ns2.nspname, t2.relname,
                 t1.relkind, t2.relkind, c.conrelid, c.conkey
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

        let column_pairs: Vec<(String, String)> = columns
            .into_iter()
            .zip(foreign_columns.into_iter())
            .collect();

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
                columns: reverse_columns,
                is_parent: true,
            }
        } else {
            Cardinality::O2M {
                constraint: constraint_name.clone(),
                columns: column_pairs,
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

    Ok(relationships)
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
            CASE
                WHEN p.proretset THEN 'SETOF ' || pg_catalog.format_type(p.prorettype, NULL)
                ELSE pg_catalog.format_type(p.prorettype, NULL)
            END as return_type,
            p.proretset as returns_set
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

        let routine = Routine {
            schema,
            name,
            description: row.get("description"),
            params: vec![], // Simplified - full implementation would parse args
            return_type,
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

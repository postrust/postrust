//! SQL queries for schema introspection.

use super::relationship::{
    Cardinality, MediaHandler, MediaHandlerMap, Relationship, RelationshipsMap,
};
use super::routine::{FuncVolatility, RetType, Routine, RoutineMap, RoutineParam};
use super::table::{Column, ColumnMap, Table, TablesMap};
use crate::api_request::QualifiedIdentifier;
use crate::db::Tx;
use crate::error::{Error, Result};
use indexmap::IndexMap;
use postrust_sql::SqlParam;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

/// Get PostgreSQL version.
pub async fn get_pg_version<T: Tx>(tx: &mut T) -> Result<i32> {
    let rows = catalog(
        tx,
        "SELECT current_setting('server_version_num')::int as version",
        &[],
        &[],
    )
    .await?;
    match rows.first() {
        Some(row) => row.get("version"),
        None => Err(Error::SchemaCacheLoadFailed(
            "no rows returned by a query that expected to return at least one row".into(),
        )),
    }
}

/// A row of a catalog query, as PostgreSQL's own `row_to_json` rendered it.
///
/// The catalog is read through the same driver-neutral interface as requests
/// (`crate::db`), whose drivers decode the types a response carries. Catalog
/// rows carry others -- `name[]`, `text[]` -- so each query is wrapped to
/// return its row as one `json` value and PostgreSQL does the decoding. A
/// driver then needs nothing but `json` to load a schema cache.
pub(super) struct CatalogRow(serde_json::Map<String, serde_json::Value>);

impl CatalogRow {
    /// A column, by name. A value of the wrong type fails the load rather than
    /// the process.
    ///
    /// A column the query does not have is an error too, not a null.
    /// `row_to_json` writes every column, null ones included, so an absent key
    /// is a name misspelled here or in the query's alias -- and read as null it
    /// would turn every `Option` field it fills into `None`, silently.
    pub(super) fn get<V: serde::de::DeserializeOwned>(&self, column: &str) -> Result<V> {
        let value = self.0.get(column).ok_or_else(|| {
            Error::SchemaCacheLoadFailed(format!("catalog query has no column \"{column}\""))
        })?;
        V::deserialize(value)
            .map_err(|e| Error::SchemaCacheLoadFailed(format!("catalog column \"{column}\": {e}")))
    }
}

/// Run a catalog query, reading each row as [`CatalogRow`], ordered by the
/// query's output columns `order_by`. Empty where the order does not matter.
///
/// The query's own `ORDER BY` is not enough. It sorts the subquery the
/// wrapper reads from, and SQL promises nothing about the order a query
/// reads a sorted subquery in. PostgreSQL keeps it today, but where a row
/// order reaches the client (see `load_relationships`) that is not a thing to
/// lean on. The planner sees that the subquery is already in this order, so
/// asking again costs no second sort.
async fn catalog<T: Tx>(
    tx: &mut T,
    sql: &str,
    order_by: &[&str],
    params: &[SqlParam],
) -> Result<Vec<CatalogRow>> {
    tx.fetch_values(&catalog_sql(sql, order_by), params)
        .await
        .map_err(|e| Error::SchemaCacheLoadFailed(e.to_string()))?
        .into_iter()
        .map(|row| match row.into_iter().next() {
            Some(serde_json::Value::Object(map)) => Ok(CatalogRow(map)),
            other => Err(Error::SchemaCacheLoadFailed(format!(
                "a catalog row was not an object: {other:?}"
            ))),
        })
        .collect()
}

/// A catalog query, wrapped to return each row as one `json` value, in the
/// order of its output columns `order_by`. See [`catalog`].
fn catalog_sql(sql: &str, order_by: &[&str]) -> String {
    let mut wrapped = format!("SELECT row_to_json(pgrst_catalog) FROM ({sql}) AS pgrst_catalog");
    for (i, column) in order_by.iter().enumerate() {
        wrapped.push_str(if i == 0 { " ORDER BY " } else { ", " });
        wrapped.push_str("pgrst_catalog.");
        wrapped.push_str(column);
    }
    wrapped
}

/// A text parameter.
fn text(value: &str) -> SqlParam {
    SqlParam::Text(value.to_string())
}

/// A `text[]` parameter.
fn texts(values: &[String]) -> SqlParam {
    SqlParam::Array(values.iter().cloned().map(SqlParam::Text).collect())
}

/// Whether a role this connection can become holds one of `privileges` in the
/// ACL `acl`, or `PUBLIC` does.
///
/// This is the test `information_schema` makes, with one word changed. Its
/// views ask `pg_has_role(role, 'USAGE')` -- does this connection hold the
/// role's privileges *without* switching to it -- which is false of every role
/// granted to a `NOINHERIT` authenticator. That is the setup PostgREST
/// recommends, and the one where the API runs every request after `SET ROLE`:
/// what the authenticator could do *as itself* is beside the point, and with
/// `USAGE` the schema cache saw no tables at all. `MEMBER` asks whether the
/// connection can become the role, which is what a request does.
///
/// For a superuser, or an authenticator that inherits, the two answers are
/// the same, so nothing about those deployments changes.
///
fn acl_grants(acl: &str, privileges: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM aclexplode({acl}) g \
          WHERE g.privilege_type = ANY ('{{{privileges}}}'::text[]) \
            AND {grantee})",
        grantee = can_become("g.grantee"),
    )
}

/// Whether this connection can become the role `role` (an OID expression), or
/// `role` is `PUBLIC`, which every role is.
///
/// `CASE` rather than `OR`: SQL does not promise to short-circuit, and
/// `pg_has_role` refuses OID 0, which is how an ACL spells `PUBLIC`.
fn can_become(role: &str) -> String {
    format!("CASE WHEN {role} = 0 THEN true ELSE pg_has_role({role}, 'MEMBER') END")
}

/// Whether this connection can become a predefined role that holds privileges
/// on every table without appearing in any ACL: `pg_read_all_data` and
/// `pg_write_all_data` (PostgreSQL 14 and later). `has_table_privilege`, which
/// `information_schema` asks, counts them, so a scan of the ACLs alone hid
/// every table from a deployment that grants one of these instead of
/// per-table privileges. Looked up by name, so on a server too old to have
/// them this is simply false rather than an error.
const PREDEFINED_DATA_ROLE: &str = "EXISTS (SELECT 1 FROM pg_roles pr \
     WHERE pr.rolname IN ('pg_read_all_data', 'pg_write_all_data') \
       AND pg_has_role(pr.oid, 'MEMBER'))";

/// A relation's own ACL, or the owner-only default it stands for when unset.
const RELATION_ACL: &str = "COALESCE(c.relacl, acldefault('r', c.relowner))";

/// Whether a relation is visible to this connection: `information_schema.tables`'
/// condition, with `MEMBER` for `USAGE` (see `acl_grants`). A role it can
/// become owns it, or holds a privilege on it or on one of its columns, or
/// reads or writes every table (see `PREDEFINED_DATA_ROLE`).
fn relation_visible() -> String {
    format!(
        "(pg_has_role(c.relowner, 'MEMBER') \
          OR {PREDEFINED_DATA_ROLE} \
          OR {table} \
          OR EXISTS (SELECT 1 FROM pg_attribute ca \
                      WHERE ca.attrelid = c.oid AND ca.attnum > 0 \
                        AND NOT ca.attisdropped AND ca.attacl IS NOT NULL \
                        AND {column}))",
        table = acl_grants(
            RELATION_ACL,
            "SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER"
        ),
        column = acl_grants("ca.attacl", "SELECT,INSERT,UPDATE,REFERENCES"),
    )
}

/// Whether a privilege on a relation is held by anyone this connection can
/// act for: `information_schema.table_privileges`' condition -- the grant is
/// visible when its grantor or its grantee is a role this connection can
/// become, or the grantee is `PUBLIC` -- with `MEMBER` for `USAGE`.
fn privilege_granted(privilege: &str) -> String {
    format!(
        "(c.relkind IN ('r', 'v', 'f', 'p') AND EXISTS (\
            SELECT 1 FROM aclexplode({RELATION_ACL}) g \
             WHERE g.privilege_type = '{privilege}' \
               AND ({grantee} OR pg_has_role(g.grantor, 'MEMBER'))))",
        grantee = can_become("g.grantee"),
    )
}

/// Load all tables and their columns.
pub async fn load_tables<T: Tx>(tx: &mut T, schemas: &[String]) -> Result<TablesMap> {
    let mut tables = HashMap::new();

    // From the catalogue rather than `information_schema`, whose views show a
    // connection only what it can use without switching role -- nothing, for
    // a NOINHERIT authenticator. See `acl_grants`. Built once: it does not
    // change between loads.
    static SQL: LazyLock<String> = LazyLock::new(|| {
        format!(
            r#"
        SELECT
            t.table_schema,
            t.table_name,
            t.table_type,
            pg_catalog.obj_description(t.oid, 'pg_class') as description,
            COALESCE(
                (SELECT array_agg(a.attname::text ORDER BY array_position(i.indkey, a.attnum))
                FROM pg_index i
                JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey)
                WHERE i.indrelid = t.oid
                  AND i.indisprimary),
                ARRAY[]::text[]
            ) as pk_cols,
            -- Being granted INSERT on a view does not make the view
            -- insertable. A view is written through only where it is simple
            -- enough for PostgreSQL to write through automatically, or where
            -- an INSTEAD OF trigger says how -- `projects_view_without_triggers`
            -- carries every grant and accepts nothing. Both halves have to
            -- hold. A table is not a view, so the grant is the whole answer
            -- there.
            --
            -- The view half is `information_schema.views`' own: the bits of
            -- `pg_relation_is_updatable` it reads, and the INSTEAD OF trigger
            -- types (`tgtype` 69 insert, 81 update, 73 delete).
            {insert} AND (
                t.relkind <> 'v'
                OR (pg_relation_is_updatable(t.oid, false) & 8) = 8
                OR EXISTS (SELECT 1 FROM pg_trigger tg
                            WHERE tg.tgrelid = t.oid AND (tg.tgtype::int & 69) = 69)
            ) as insertable,
            {update} AND (
                t.relkind <> 'v'
                OR (pg_relation_is_updatable(t.oid, false) & 20) = 20
                OR EXISTS (SELECT 1 FROM pg_trigger tg
                            WHERE tg.tgrelid = t.oid AND (tg.tgtype::int & 81) = 81)
            ) as updatable,
            -- Updatable rather than a deletable bit, as `information_schema`
            -- has it: a view PostgreSQL can write through automatically is
            -- one it can delete through as well. Only the trigger half is
            -- asked separately.
            {delete} AND (
                t.relkind <> 'v'
                OR (pg_relation_is_updatable(t.oid, false) & 20) = 20
                OR EXISTS (SELECT 1 FROM pg_trigger tg
                            WHERE tg.tgrelid = t.oid AND (tg.tgtype::int & 73) = 73)
            ) as deletable,
            t.relkind = 'p' as is_partitioned
        FROM (
            -- A foreign table is a table: it has columns, it can be selected
            -- from and written to, and which server stores the rows is not
            -- the API's business. Leaving it out meant a schema that exposes
            -- one answered "no such table".
            --
            -- A materialized view is a view that cannot be written to, which
            -- is what the privilege tests report for it -- they cover only
            -- the relation kinds `information_schema.table_privileges` does,
            -- and this is not one of them.
            SELECT c.oid, c.relkind, c.relispartition,
                   n.nspname::text AS table_schema,
                   c.relname::text AS table_name,
                   CASE WHEN c.relkind IN ('v', 'm') THEN 'VIEW'
                        WHEN c.relkind = 'f' THEN 'FOREIGN'
                        ELSE 'BASE TABLE' END AS table_type,
                   {visible} OR c.relkind = 'm' AS visible
              FROM pg_class c
              JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE c.relkind IN ('r', 'v', 'f', 'p', 'm')
               AND NOT pg_is_other_temp_schema(n.oid)
               AND n.oid <> pg_my_temp_schema()
        ) t
        JOIN pg_class c ON c.oid = t.oid
        WHERE t.table_schema = ANY($1)
          AND t.visible
          -- A partition is reached through the table it partitions, not on its
          -- own. Listing every one of them gave a schema with a year of
          -- monthly partitions twelve endpoints PostgREST does not have -- and
          -- a request naming one was answered as though the table were real,
          -- which made a missing relationship read as a missing column. The
          -- partitioned parent itself (`relkind = 'p'`) is not a partition and
          -- stays.
          AND NOT t.relispartition
        ORDER BY t.table_schema, t.table_name
        "#,
            insert = privilege_granted("INSERT"),
            update = privilege_granted("UPDATE"),
            delete = privilege_granted("DELETE"),
            visible = relation_visible(),
        )
    });
    let rows = catalog(tx, &SQL, &["table_schema", "table_name"], &[texts(schemas)]).await?;

    for row in rows {
        let schema: String = row.get("table_schema")?;
        let name: String = row.get("table_name")?;
        let qi = QualifiedIdentifier::new(&schema, &name);

        let table_type: String = row.get("table_type")?;
        let pk_cols: Vec<String> = row.get("pk_cols")?;

        let table = Table {
            schema: schema.clone(),
            name: name.clone(),
            description: row.get("description")?,
            is_view: table_type == "VIEW",
            is_partitioned: row.get("is_partitioned")?,
            insertable: row.get("insertable")?,
            updatable: row.get("updatable")?,
            deletable: row.get("deletable")?,
            pk_cols: pk_cols.clone(),
            unique_constraints: Vec::new(),
            columns: load_columns(tx, &schema, &name, &pk_cols).await?,
            computed_columns: Default::default(),
        };

        tables.insert(qi, table);
    }

    Ok(tables)
}

/// Load columns for a table.
async fn load_columns<T: Tx>(
    tx: &mut T,
    schema: &str,
    table: &str,
    pk_cols: &[String],
) -> Result<ColumnMap> {
    let mut columns = IndexMap::new();

    // `information_schema.columns`, read from the catalogue: the view's own
    // expressions for every column used here, so a type is named exactly as it
    // was, and its visibility test with `MEMBER` for `USAGE` -- see
    // `acl_grants` for why the view itself shows a NOINHERIT authenticator
    // nothing. Built once, as it runs once per table.
    static SQL: LazyLock<String> = LazyLock::new(|| {
        format!(
            r#"
        SELECT
            c.column_name,
            c.ordinal_position,
            c.is_nullable,
            c.data_type,
            c.udt_name,
            -- A domain's own name. `information_schema` reports the type
            -- underneath it in `data_type` and `udt_name`, so this is the only
            -- column that says a value is a `color` rather than an integer --
            -- and the domain is what a data representation is declared on.
            c.domain_name,
            c.character_maximum_length,
            -- An identity column has no `column_default`: `information_schema`
            -- describes it as generated rather than defaulted. What it would
            -- take if a row left it out is the next value of the sequence the
            -- column owns, which is the answer `Prefer: missing=default`
            -- needs.
            COALESCE(
                c.column_default,
                (SELECT format('nextval(%L)', s.oid::regclass)
                   FROM pg_class rel
                   JOIN pg_namespace ns ON ns.oid = rel.relnamespace
                   JOIN pg_attribute att ON att.attrelid = rel.oid
                                        AND att.attname = c.column_name
                                        AND att.attidentity = 'd'
                   JOIN pg_depend dep ON dep.refobjid = rel.oid
                                     AND dep.refobjsubid = att.attnum
                                     AND dep.deptype = 'i'
                   JOIN pg_class s ON s.oid = dep.objid AND s.relkind = 'S'
                  WHERE ns.nspname = c.table_schema
                    AND rel.relname = c.table_name
                  LIMIT 1)
            ) AS column_default,
            -- What a write may not name at all. `GENERATED ALWAYS AS
            -- IDENTITY` and a stored generated column both belong to
            -- PostgreSQL, which answers `cannot insert a non-DEFAULT value`
            -- to anything that names one. `BY DEFAULT` is deliberately not
            -- here: it has a default, and a value given for it is taken.
            COALESCE(
                (c.is_identity = 'YES' AND c.identity_generation = 'ALWAYS')
                    OR c.is_generated = 'ALWAYS',
                false
            ) AS always_generated,
            -- By OID: naming the table and casting to `regclass` needs USAGE
            -- on its schema, which a NOINHERIT authenticator does not have.
            pg_catalog.col_description(c.relid, c.ordinal_position) as description,
            CASE WHEN e.enumtypid IS NOT NULL
                 THEN array_agg(e.enumlabel ORDER BY e.enumsortorder)
                 ELSE ARRAY[]::text[]
            END as enum_values
        FROM (
            SELECT
                c.oid AS relid,
                nc.nspname::text AS table_schema,
                c.relname::text AS table_name,
                a.attname::text AS column_name,
                a.attnum::int AS ordinal_position,
                CASE WHEN a.attgenerated = '' THEN pg_get_expr(ad.adbin, ad.adrelid) END
                    AS column_default,
                CASE WHEN a.attnotnull OR (t.typtype = 'd' AND t.typnotnull)
                     THEN 'NO' ELSE 'YES' END AS is_nullable,
                CASE WHEN t.typtype = 'd' THEN
                         CASE WHEN bt.typelem <> 0 AND bt.typlen = -1 THEN 'ARRAY'
                              WHEN nbt.nspname = 'pg_catalog' THEN format_type(t.typbasetype, NULL)
                              ELSE 'USER-DEFINED' END
                     ELSE
                         CASE WHEN t.typelem <> 0 AND t.typlen = -1 THEN 'ARRAY'
                              WHEN nt.nspname = 'pg_catalog' THEN format_type(a.atttypid, NULL)
                              ELSE 'USER-DEFINED' END
                END AS data_type,
                information_schema._pg_char_max_length(
                    information_schema._pg_truetypid(a.*, t.*),
                    information_schema._pg_truetypmod(a.*, t.*)
                )::int AS character_maximum_length,
                CASE WHEN t.typtype = 'd' THEN t.typname::text END AS domain_name,
                COALESCE(bt.typname, t.typname)::text AS udt_name,
                -- The same type by OID, for its enum labels: two schemas may
                -- each have an enum of one name.
                COALESCE(bt.oid, t.oid) AS udt_oid,
                CASE WHEN a.attidentity IN ('a', 'd') THEN 'YES' ELSE 'NO' END AS is_identity,
                CASE a.attidentity WHEN 'a' THEN 'ALWAYS' WHEN 'd' THEN 'BY DEFAULT' END
                    AS identity_generation,
                CASE WHEN a.attgenerated <> '' THEN 'ALWAYS' ELSE 'NEVER' END AS is_generated
            FROM pg_attribute a
            LEFT JOIN pg_attrdef ad ON a.attrelid = ad.adrelid AND a.attnum = ad.adnum
            JOIN pg_class c ON a.attrelid = c.oid
            JOIN pg_namespace nc ON c.relnamespace = nc.oid
            JOIN pg_type t ON a.atttypid = t.oid
            JOIN pg_namespace nt ON t.typnamespace = nt.oid
            LEFT JOIN (pg_type bt JOIN pg_namespace nbt ON bt.typnamespace = nbt.oid)
                   ON t.typtype = 'd' AND t.typbasetype = bt.oid
            WHERE nc.nspname = $1 AND c.relname = $2
              AND NOT pg_is_other_temp_schema(nc.oid)
              AND a.attnum > 0 AND NOT a.attisdropped
              AND c.relkind IN ('r', 'v', 'f', 'p')
              AND (pg_has_role(c.relowner, 'MEMBER')
                   OR {predefined}
                   OR {table_grant}
                   OR (a.attacl IS NOT NULL AND {column_grant}))
        ) c
        LEFT JOIN pg_enum e ON e.enumtypid = c.udt_oid
        WHERE c.table_schema = $1 AND c.table_name = $2
        GROUP BY c.relid, c.table_schema, c.table_name, c.column_name, c.ordinal_position, c.is_nullable,
                 c.data_type, c.udt_name, c.domain_name, c.character_maximum_length,
                 c.column_default, c.is_identity, c.identity_generation, c.is_generated,
                 c.udt_oid, e.enumtypid
        ORDER BY c.ordinal_position
        "#,
            predefined = PREDEFINED_DATA_ROLE,
            table_grant = acl_grants(RELATION_ACL, "SELECT,INSERT,UPDATE,REFERENCES"),
            column_grant = acl_grants("a.attacl", "SELECT,INSERT,UPDATE,REFERENCES"),
        )
    });
    let rows = catalog(
        tx,
        &SQL,
        &["ordinal_position"],
        &[text(schema), text(table)],
    )
    .await?;

    // A materialized view has no rows in `information_schema.columns`; its
    // columns come from the catalogue instead.
    let rows = match rows.is_empty() {
        true => load_catalog_columns(tx, schema, table).await?,
        false => rows,
    };

    for row in rows {
        let name: String = row.get("column_name")?;
        let is_nullable: String = row.get("is_nullable")?;
        let data_type: String = row.get("data_type")?;
        let udt_name: String = row.get("udt_name")?;
        let max_len: Option<i32> = row.get("character_maximum_length")?;
        let enum_values: Vec<String> = row.get("enum_values")?;
        let position: i32 = row.get("ordinal_position")?;

        let column = Column {
            name: name.clone(),
            description: row.get("description")?,
            nullable: is_nullable == "YES",
            data_type,
            nominal_type: udt_name,
            domain_type: row.get("domain_name")?,
            max_len,
            default: row.get("column_default")?,
            enum_values,
            is_pk: pk_cols.contains(&name),
            position,
            always_generated: row.get("always_generated")?,
        };

        columns.insert(name, column);
    }

    Ok(columns)
}

/// Load foreign key relationships.
pub async fn load_relationships<T: Tx>(tx: &mut T, schemas: &[String]) -> Result<RelationshipsMap> {
    let mut relationships: RelationshipsMap = HashMap::new();

    let rows = catalog(
        tx,
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
        -- Ordered, because the order reaches the client. Where two foreign
        -- keys join the same pair of tables the embed is ambiguous, and the
        -- PGRST201 that says so lists the candidates -- in this order, both in
        -- `details` and in the `hint` that tells the caller what to write
        -- instead. `pg_constraint` unordered is heap order, which differs
        -- between two databases holding the same schema and can differ between
        -- two reads of one database. That made the error message arbitrary and
        -- the conformance cases covering it flip between runs.
        ORDER BY ns1.nspname, t1.relname, c.conname
        "#,
        &["table_schema", "table_name", "constraint_name"],
        &[texts(schemas)],
    )
    .await?;

    for row in rows {
        let table_schema: String = row.get("table_schema")?;
        let table_name: String = row.get("table_name")?;
        let foreign_schema: String = row.get("foreign_table_schema")?;
        let foreign_name: String = row.get("foreign_table_name")?;
        let constraint_name: String = row.get("constraint_name")?;
        let columns: Vec<String> = row.get("columns")?;
        let foreign_columns: Vec<String> = row.get("foreign_columns")?;
        let table_is_view: bool = row.get("table_is_view")?;
        let foreign_is_view: bool = row.get("foreign_table_is_view")?;
        let is_unique: bool = row.get("is_unique")?;

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

    load_computed_relationships(tx, schemas, &mut relationships).await?;

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
async fn load_computed_relationships<T: Tx>(
    tx: &mut T,
    schemas: &[String],
    relationships: &mut RelationshipsMap,
) -> Result<()> {
    let rows = catalog(
        tx,
        r#"
        SELECT
            pn.nspname  AS function_schema,
            p.proname   AS function_name,
            tn.nspname  AS table_schema,
            t.relname   AS table_name,
            fn.nspname  AS foreign_table_schema,
            f.relname   AS foreign_table_name,
            (NOT p.proretset) OR p.prorows = 1 AS to_one,
            (SELECT p.proargnames[i]
               FROM generate_series(1, p.pronargs) AS i
              WHERE p.proargtypes[i - 1] = argt.oid
              LIMIT 1) AS row_argument
        FROM pg_proc p
        JOIN pg_namespace pn ON pn.oid = p.pronamespace
        -- one argument is a table's composite type. Usually the only one, and
        -- then the call is positional; a function that also takes a search
        -- term or the caller's session has it somewhere among the rest, and is
        -- called by name.
        JOIN pg_type argt ON argt.oid = ANY (p.proargtypes::oid[])
        JOIN pg_class t   ON t.oid = argt.typrelid
        JOIN pg_namespace tn ON tn.oid = t.relnamespace
        -- and the return type is a table as well
        JOIN pg_type rett ON rett.oid = p.prorettype
        JOIN pg_class f   ON f.oid = rett.typrelid
        JOIN pg_namespace fn ON fn.oid = f.relnamespace
        -- Every argument named, since one that is not cannot be given by name
        -- and a call with more than the row has to be.
        WHERE (p.pronargs = 1 OR (SELECT count(*)
                                    FROM generate_series(1, p.pronargs) AS i
                                   WHERE COALESCE(p.proargnames[i], '') = '') = 0)
          AND pn.nspname = ANY($1)
          AND tn.nspname = ANY($1)
          AND fn.nspname = ANY($1)
          -- relkind 'c' would be a standalone composite type, not a relation
          AND t.relkind = ANY (ARRAY['r','v','m','f','p'])
          AND f.relkind = ANY (ARRAY['r','v','m','f','p'])
        "#,
        &[],
        &[texts(schemas)],
    )
    .await?;

    for row in rows {
        let function = QualifiedIdentifier::new(
            row.get::<String>("function_schema")?,
            row.get::<String>("function_name")?,
        );
        let table_schema: String = row.get("table_schema")?;
        let table = QualifiedIdentifier::new(&table_schema, row.get::<String>("table_name")?);
        let foreign_table = QualifiedIdentifier::new(
            row.get::<String>("foreign_table_schema")?,
            row.get::<String>("foreign_table_name")?,
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
                to_one: row.get("to_one")?,
                is_self,
                row_argument: row.get::<Option<String>>("row_argument")?,
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
                // A parameter declared `char(4)` is stored as bare
                // `character`, whose length is 1, so casting an argument to
                // the declared type truncates it. PostgreSQL does not enforce
                // a length on a function parameter at all, so the unbounded
                // spelling is the honest one to cast to.
                type_max_length: item
                    .get("cast_type")
                    .and_then(|t| t.as_str())
                    .unwrap_or(&param_type)
                    .to_string(),
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
pub async fn load_routines<T: Tx>(tx: &mut T, schemas: &[String]) -> Result<RoutineMap> {
    let mut routines: RoutineMap = HashMap::new();

    let rows = catalog(
        tx,
        r#"
        -- What a type is once every domain over it is stripped away.
        --
        -- `RETURNS SETOF projects_domain` returns rows of `projects`, and a
        -- domain is transparent to everything that matters here: whether the
        -- result is composite, and which table's columns it has. Reading the
        -- declared type alone finds a type name that names no table.
        WITH RECURSIVE base_types AS (
            SELECT oid, typbasetype, oid AS base_type FROM pg_catalog.pg_type
            UNION
            SELECT t.oid, b.typbasetype, b.oid AS base_type
              FROM base_types t
              JOIN pg_catalog.pg_type b ON b.oid = t.typbasetype
        ),
        resolved AS (
            SELECT oid, base_type FROM base_types WHERE typbasetype = 0
        )
        SELECT
            n.nspname as schema,
            p.proname as name,
            pg_catalog.obj_description(p.oid) as description,
            p.provolatile::text as volatility,
            p.provariadic <> 0 as has_variadic,
            EXISTS (
                SELECT 1 FROM unnest(COALESCE(p.proargmodes, '{}'::"char"[])) AS mode
                 WHERE mode IN ('o', 't', 'b')
            ) as has_out_params,
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
                        -- What an argument is cast to. `character` and `bit`
                        -- name themselves without a length, and casting to
                        -- either truncates to one character; a parameter's
                        -- length is not enforced anyway, so the varying
                        -- spelling is what the value should reach.
                        'cast_type', CASE p.proargtypes[i - 1]
                            WHEN 'character'::regtype   THEN 'character varying'
                            WHEN 'character[]'::regtype THEN 'character varying[]'
                            WHEN 'bit'::regtype         THEN 'bit varying'
                            WHEN 'bit[]'::regtype       THEN 'bit varying[]'
                            ELSE pg_catalog.format_type(p.proargtypes[i - 1], NULL)
                        END,
                        'required', i <= (p.pronargs - p.pronargdefaults),
                        'variadic', p.provariadic <> 0 AND i = p.pronargs
                    ) ORDER BY i
                )
                FROM generate_series(1, p.pronargs) AS i
            ), '[]'::json) as params,
            -- The columns the function's rows have, where it names them:
            -- `RETURNS TABLE(a text, b colour)` declares them as OUT
            -- parameters, and they are the only account of the result's shape
            -- -- there is no table to read it off. `proallargtypes` is a
            -- plain 1-indexed array and lines up with `proargnames`.
            COALESCE((
                SELECT json_agg(
                    json_build_object(
                        'name', COALESCE(p.proargnames[i], ''),
                        'type', pg_catalog.format_type(p.proallargtypes[i], NULL)
                    ) ORDER BY i
                )
                FROM generate_series(
                    1, COALESCE(array_length(p.proallargtypes, 1), 0)) AS i
                WHERE p.proargmodes[i] IN ('o', 't', 'b')
            ), '[]'::json) as out_params,
            CASE
                WHEN p.proretset THEN 'SETOF ' || pg_catalog.format_type(rt.base_type, NULL)
                ELSE pg_catalog.format_type(rt.base_type, NULL)
            END as return_type,
            p.proretset as returns_set,
            (SELECT t.typtype::text FROM pg_catalog.pg_type t WHERE t.oid = rt.base_type)
                as ret_typtype,
            -- A domain named after a media type says the value already is
            -- that media type: `returns "image/png"` is a PNG, not a row to
            -- serialise. Only a single value can be one, so a set-returning
            -- function is not it.
            (SELECT lower(dt.typname) FROM pg_catalog.pg_type dt
              WHERE dt.oid = p.prorettype
                AND dt.typbasetype <> 0
                AND NOT p.proretset
                AND (dt.typname ~ '^[A-Za-z0-9.-]+/[A-Za-z0-9.+-]+$'
                     OR dt.typname = '*/*')) as media_type,
            -- What that domain is a domain *over*. A value whose type this
            -- side does not recognise arrives as PostgreSQL's text rendering
            -- of it, and a `bytea` renders as `\x` and hex -- which is a
            -- description of the bytes, not the bytes. Knowing the base type
            -- is what tells the two apart.
            (SELECT lower(bt.typname) FROM pg_catalog.pg_type dt
               JOIN pg_catalog.pg_type bt ON bt.oid = dt.typbasetype
              WHERE dt.oid = p.prorettype
                AND NOT p.proretset
                AND (dt.typname ~ '^[A-Za-z0-9.-]+/[A-Za-z0-9.+-]+$'
                     OR dt.typname = '*/*')) as media_base_type
        FROM pg_proc p
        JOIN pg_namespace n ON n.oid = p.pronamespace
        JOIN resolved rt ON rt.oid = p.prorettype
        WHERE n.nspname = ANY($1)
          AND p.prokind IN ('f', 'p')
          -- A parameter with no name cannot be given an argument by name, so
          -- a function that has one is not callable over the API at all --
          -- and leaving it in the cache makes it a candidate that no request
          -- can ever match, and a hint that sends the client nowhere.
          --
          -- The exception is the single unnamed parameter that takes the
          -- whole request body, which is how a function is called with one
          -- json, text, xml or bytea argument and no name for it.
          AND (
            (SELECT count(*) FROM generate_series(1, p.pronargs) AS i
              WHERE COALESCE(p.proargnames[i], '') = '') = 0
            OR (
              p.pronargs = 1
              AND COALESCE(p.proargnames[1], '') = ''
              AND p.proargtypes[0] IN ('bytea'::regtype, 'json'::regtype,
                                       'jsonb'::regtype, 'text'::regtype,
                                       'xml'::regtype)
            )
          )
        ORDER BY n.nspname, p.proname
        "#,
        &["schema", "name"],
        &[texts(schemas)],
    )
    .await?;

    for row in rows {
        let schema: String = row.get("schema")?;
        let name: String = row.get("name")?;
        let qi = QualifiedIdentifier::new(&schema, &name);

        let volatility: String = row.get("volatility")?;
        let return_type_str: String = row.get("return_type")?;
        let returns_set: bool = row.get("returns_set")?;

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
        let ret_typtype: Option<String> = row.get("ret_typtype")?;
        // A named row type always expands. `record` expands only when the
        // function declares its columns as `OUT` parameters or a `RETURNS
        // TABLE` -- a bare `RETURNS record` has none, and PostgreSQL refuses
        // to put one in a `FROM` clause without being told what they are.
        let has_out_params: bool = row.get("has_out_params")?;
        let returns_composite = !matches!(return_type, RetType::Void)
            && (ret_typtype.as_deref() == Some("c")
                || (ret_typtype.as_deref() == Some("p") && has_out_params));

        let routine = Routine {
            schema,
            name,
            description: row.get("description")?,
            params: parse_routine_params(row.get("params")?),
            return_type,
            returns_composite,
            volatility: FuncVolatility::from_char(volatility.chars().next().unwrap_or('v')),
            has_variadic: row.get("has_variadic")?,
            isolation_level: None,
            settings: vec![],
            is_procedure: row.get("is_procedure")?,
            media_type: row.get("media_type")?,
            media_base_type: row.get("media_base_type")?,
            output_columns: parse_routine_params(row.get("out_params")?)
                .into_iter()
                .filter(|param| !param.name.is_empty())
                .map(|param| (param.name, param.param_type))
                .collect(),
        };

        routines.entry(qi).or_default().push(routine);
    }

    // Overloads of one name in a fixed order: fewest parameters first, then by
    // the parameters themselves. Catalogue order is whatever the planner
    // happened to return, and it decides which signature a client is offered
    // first when a call cannot be told apart -- an answer that should not
    // change between two servers reading the same schema.
    for overloads in routines.values_mut() {
        overloads.sort_by(|a, b| {
            let key = |r: &Routine| {
                (
                    r.params.len(),
                    r.params
                        .iter()
                        .map(|p| (p.name.clone(), p.param_type.clone()))
                        .collect::<Vec<_>>(),
                )
            };
            key(a).cmp(&key(b))
        });
    }

    Ok(routines)
}

/// Load valid timezone names.
pub async fn load_timezones<T: Tx>(tx: &mut T) -> Result<HashSet<String>> {
    let rows = catalog(tx, "SELECT name FROM pg_timezone_names", &[], &[]).await?;

    rows.iter().map(|r| r.get("name")).collect()
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
pub async fn load_media_handlers<T: Tx>(tx: &mut T, schemas: &[String]) -> Result<MediaHandlerMap> {
    let rows = catalog(
        tx,
        r#"
        SELECT
            n.nspname    AS agg_schema,
            p.proname    AS agg_name,
            t.typname    AS media_type,
            bt.typname   AS media_base_type,
            argn.nspname AS arg_schema,
            argc.relname AS arg_table
        FROM pg_aggregate a
        JOIN pg_proc p       ON p.oid = a.aggfnoid
        JOIN pg_namespace n  ON n.oid = p.pronamespace
        JOIN pg_type t       ON t.oid = a.aggtranstype
        JOIN pg_type bt      ON bt.oid = t.typbasetype
        LEFT JOIN pg_type argt      ON argt.oid = p.proargtypes[0]
        LEFT JOIN pg_class argc     ON argc.oid = argt.typrelid
        LEFT JOIN pg_namespace argn ON argn.oid = argc.relnamespace
        WHERE n.nspname = ANY($1)
          AND t.typtype = 'd'
          AND t.typname LIKE '%/%'
        "#,
        &[],
        &[texts(schemas)],
    )
    .await?;

    let mut handlers: MediaHandlerMap = HashMap::new();
    for row in rows {
        let agg_schema: String = row.get("agg_schema")?;
        let media_type: String = row.get("media_type")?;
        let table = match (
            row.get::<Option<String>>("arg_schema")?,
            row.get::<Option<String>>("arg_table")?,
        ) {
            (Some(schema), Some(name)) => Some(QualifiedIdentifier::new(schema, name)),
            _ => None,
        };

        handlers
            .entry((agg_schema.clone(), media_type))
            .or_default()
            .push(MediaHandler {
                aggregate: QualifiedIdentifier::new(agg_schema, row.get::<String>("agg_name")?),
                table,
                base_type: row.get("media_base_type")?,
            });
    }

    Ok(handlers)
}

/// Which base-table column each of a view's columns comes from.
///
/// A view has no foreign keys of its own, but it can still be embedded: the
/// relationships of the tables it selects from apply to it, through whichever
/// of their columns it carries.
///
/// The mapping is made by name. PostgreSQL records which base columns a view
/// depends on, but not which of the view's own columns each one became -- that
/// lives in the parsed rule tree. Matching by name covers a view that selects
/// its columns through unchanged, and misses one that renames them, which is
/// Columns read straight from the catalogue, for a relation
/// `information_schema` does not describe.
///
/// The columns are shaped to match what the `information_schema` query
/// returns, so the caller cannot tell which of the two answered.
async fn load_catalog_columns<T: Tx>(
    tx: &mut T,
    schema: &str,
    table: &str,
) -> Result<Vec<CatalogRow>> {
    catalog(
        tx,
        r#"
        SELECT
            a.attname AS column_name,
            a.attnum::int AS ordinal_position,
            CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END AS is_nullable,
            pg_catalog.format_type(a.atttypid, NULL) AS data_type,
            t.typname AS udt_name,
            CASE WHEN t.typtype = 'd' THEN t.typname END AS domain_name,
            NULL::int AS character_maximum_length,
            pg_catalog.pg_get_expr(d.adbin, d.adrelid) AS column_default,
            (a.attidentity = 'a' OR a.attgenerated <> '') AS always_generated,
            pg_catalog.col_description(c.oid, a.attnum) AS description,
            COALESCE(
                (SELECT array_agg(e.enumlabel ORDER BY e.enumsortorder)
                   FROM pg_enum e WHERE e.enumtypid = t.oid),
                ARRAY[]::text[]
            ) AS enum_values
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped
        JOIN pg_type t ON t.oid = a.atttypid
        LEFT JOIN pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum
        WHERE n.nspname = $1 AND c.relname = $2
        ORDER BY a.attnum
        "#,
        &["ordinal_position"],
        &[text(schema), text(table)],
    )
    .await
}

/// Casts a schema declares between one of its domains and `json` or `text`.
///
/// PostgREST calls these data representations: a domain over `integer` with a
/// cast to `json` decides how a column of that domain is rendered, and a cast
/// back from `text` decides how a filter value written against it is read. It
/// lets a schema say that a colour is `"#01E240"` on the wire and an integer in
/// storage, with neither the client nor the table having to know about the
/// other.
///
/// Keyed by `(source type, target type)` so both directions come out of one
/// query: the render is `(domain, json)` and the parse is `(text, domain)`.
pub async fn load_representations<T: Tx>(
    tx: &mut T,
) -> Result<std::collections::HashMap<(String, String), String>> {
    // Whether a request may call the cast function: owned by, or executable
    // by, a role this connection can become. `has_function_privilege` asked
    // whether the connection could call it *as itself*, which a NOINHERIT
    // authenticator cannot once EXECUTE is revoked from PUBLIC -- and the
    // representation was dropped. See `acl_grants`.
    static SQL: LazyLock<String> = LazyLock::new(|| {
        format!(
            r#"
        SELECT c.castsource::regtype::text AS source,
               c.casttarget::regtype::text AS target,
               c.castfunc::regproc::text   AS function
          FROM pg_catalog.pg_cast c
          JOIN pg_catalog.pg_type src ON src.oid = c.castsource
          JOIN pg_catalog.pg_type dst ON dst.oid = c.casttarget
          JOIN pg_catalog.pg_proc p ON p.oid = c.castfunc
         WHERE c.castcontext = 'i'
           AND c.castmethod = 'f'
           AND (pg_has_role(p.proowner, 'MEMBER') OR {execute})
           AND ((src.typtype = 'd' AND c.casttarget IN ('json'::regtype, 'text'::regtype))
             OR (dst.typtype = 'd' AND c.castsource IN ('json'::regtype, 'text'::regtype)))
        "#,
            execute = acl_grants("COALESCE(p.proacl, acldefault('f', p.proowner))", "EXECUTE"),
        )
    });
    let rows = catalog(tx, &SQL, &[], &[]).await?;

    rows.into_iter()
        .map(|row| {
            Ok((
                (row.get::<String>("source")?, row.get::<String>("target")?),
                row.get::<String>("function")?,
            ))
        })
        .collect()
}

/// Functions of a table's row type, which read as though they were columns.
///
/// `create function always_true(items) returns boolean` makes
/// `?select=id,always_true` and `?always_true=is.true` work on `items`:
/// PostgreSQL resolves `items.always_true` to the function call, and PostgREST
/// exposes that. Only single-argument functions returning a single value
/// qualify -- one returning `setof` another table is a computed *relationship*,
/// which is embedded rather than selected.
/// `function_schemas` is where the function itself may live: the exposed
/// schemas plus whatever `db-extra-search-path` adds. The table has to be
/// exposed -- it is the thing being read -- but the function need not be, and
/// PostgREST resolves it along the search path. `always_false(test.items)`
/// lives in `public`, so requiring the function to be in an exposed schema
/// left `?always_false=is.false` answered "column does not exist" for a field
/// PostgREST reads as a column.
pub async fn load_computed_columns<T: Tx>(
    tx: &mut T,
    schemas: &[String],
    function_schemas: &[String],
) -> Result<Vec<ComputedColumnRow>> {
    // One argument is the table's row. The rest are the caller's, and are
    // exposed as the field's own arguments -- except the session document
    // Hasura fills in, `hasura_session json`, which a client never sends and
    // which is what lets a computed field know who is asking.
    //
    // Every argument has to be named: one that is not cannot be given by name,
    // and a call with more than the row has to be.
    let rows = catalog(
        tx,
        r#"
        SELECT tn.nspname AS table_schema,
               t.relname  AS table_name,
               fn.nspname AS function_schema,
               p.proname  AS function_name,
               pg_catalog.format_type(p.prorettype, null) AS return_type,
               pg_catalog.obj_description(p.oid, 'pg_proc') AS description,
               (SELECT p.proargnames[i]
                  FROM generate_series(1, p.pronargs) AS i
                 WHERE p.proargtypes[i - 1] = t.reltype
                 LIMIT 1) AS row_argument,
               (SELECT p.proargnames[i]
                  FROM generate_series(1, p.pronargs) AS i
                 WHERE p.proargnames[i] = 'hasura_session'
                 LIMIT 1) AS session_argument,
               p.pronargs > 1 AS takes_arguments
          FROM pg_proc p
          JOIN pg_namespace fn ON fn.oid = p.pronamespace
          JOIN pg_class t ON t.reltype = ANY (p.proargtypes::oid[])
          JOIN pg_namespace tn ON tn.oid = t.relnamespace
         WHERE (p.pronargs = 1 OR (SELECT count(*)
                                     FROM generate_series(1, p.pronargs) AS i
                                    WHERE COALESCE(p.proargnames[i], '') = '') = 0)
           AND NOT p.proretset
           AND p.prokind = 'f'
           AND p.prorettype <> 'pg_catalog.trigger'::pg_catalog.regtype
           AND t.relkind = ANY (ARRAY['r', 'v', 'm', 'p', 'f'])
           AND fn.nspname = ANY($2)
           AND tn.nspname = ANY($1)
        "#,
        &[],
        &[texts(schemas), texts(function_schemas)],
    )
    .await?;

    rows.into_iter()
        .map(|row| -> Result<_> {
            Ok(ComputedColumnRow {
                table: QualifiedIdentifier::new(
                    row.get::<String>("table_schema")?,
                    row.get::<String>("table_name")?,
                ),
                name: row.get::<String>("function_name")?,
                function: QualifiedIdentifier::new(
                    row.get::<String>("function_schema")?,
                    row.get::<String>("function_name")?,
                ),
                return_type: row.get::<String>("return_type")?,
                description: row.get::<Option<String>>("description")?,
                row_argument: row.get::<Option<String>>("row_argument")?,
                session_argument: row.get::<Option<String>>("session_argument")?,
                takes_arguments: row.get::<bool>("takes_arguments")?,
            })
        })
        .collect()
}

/// One function that reads as a column of a table.
#[derive(Clone, Debug)]
pub struct ComputedColumnRow {
    /// The table it belongs to.
    pub table: QualifiedIdentifier,
    /// The name it is read under, which is the function's.
    pub name: String,
    /// The function itself.
    pub function: QualifiedIdentifier,
    /// What it returns.
    pub return_type: String,
    /// The function's comment.
    pub description: Option<String>,
    /// The parameter that takes the row.
    pub row_argument: Option<String>,
    /// The parameter filled from the session, if any.
    pub session_argument: Option<String>,
    /// Whether it takes anything besides the row.
    pub takes_arguments: bool,
}

/// One view column, and the base-table column it was selected from.
#[derive(Clone, Debug)]
pub struct ViewColumn {
    /// The view.
    pub view: QualifiedIdentifier,
    /// The table the column ultimately comes from.
    pub base: QualifiedIdentifier,
    /// The column's name on the base table.
    pub base_column: String,
    /// The name the view gives it.
    pub view_column: String,
}

/// Which base-table column each view column came from.
///
/// A view may rename what it selects -- `select article_id as "articleId"` --
/// so matching by name gets the mapping wrong exactly where it matters, and a
/// view over another view has to be followed through. PostgreSQL records the
/// answer in the rewrite rule's target list, where every entry carries the
/// table and column it originated from, but only as a `pg_node_tree`, which
/// has no catalogue view and no parser exposed to SQL.
///
/// The shape below is PostgREST's: turn the node tree into JSON with a fixed
/// series of string replacements -- the fields wanted are quoted first so the
/// one regular expression can strip everything else -- read the target list
/// out of it, then follow view-on-view chains recursively, stopping on a cycle.
///
/// One base column may arrive under several names (`select t1_id as t1_id1,
/// t1_id as t1_id2`), so a single mapping is a row per name.
pub async fn load_view_columns<T: Tx>(tx: &mut T, schemas: &[String]) -> Result<Vec<ViewColumn>> {
    let rows = catalog(
        tx,
        r#"
        WITH RECURSIVE
        views AS (
            SELECT c.oid AS view_id, c.relnamespace AS view_schema_id,
                   n.nspname AS view_schema, c.relname AS view_name,
                   r.ev_action AS view_definition
              FROM pg_class c
              JOIN pg_namespace n ON n.oid = c.relnamespace
              JOIN pg_rewrite r ON r.ev_class = c.oid
             WHERE c.relkind IN ('v', 'm')
        ),
        transform_json AS (
            SELECT view_id, view_schema_id, view_schema, view_name,
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                regexp_replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                replace(
                    view_definition::text,
                    '<>', '()'
                ), ',', ''
                ), E'\\{', ''
                ), E'\\}', ''
                ), ' :targetList ', ',"targetList":'
                ), ' :resno ', ',"resno":'
                ), ' :resorigtbl ', ',"resorigtbl":'
                ), ' :resorigcol ', ',"resorigcol":'
                ), '{', '{ :'
                ), '((', '{(('
                ), '({', '{({'
                ), ' :[^}{,]+', ',"":', 'g'
                ), ',"":}', '}'
                ), ',"":,', ','
                ), '{(', '('
                ), '{,', '{'
                ), '(', '['
                ), ')', ']'
                ), ' ', ','
                )::json AS view_definition
              FROM views
        ),
        results AS (
            SELECT view_id, view_schema_id, view_schema, view_name,
                   (entry->>'resno')::int AS view_column,
                   (entry->>'resorigtbl')::oid AS resorigtbl,
                   (entry->>'resorigcol')::int AS resorigcol
              FROM (
                SELECT view_id, view_schema_id, view_schema, view_name,
                       json_array_elements(view_definition->0->'targetList') AS entry
                  FROM transform_json
              ) target_entries
        ),
        -- A view over a view is followed down to the base table. `path`
        -- carries the tables already visited so a cyclic definition stops
        -- rather than recursing forever.
        recursion(view_id, view_schema, view_name, view_column,
                  resorigtbl, resorigcol, is_cycle, path) AS (
            SELECT r.view_id, r.view_schema, r.view_name, r.view_column,
                   r.resorigtbl, r.resorigcol, false, ARRAY[r.resorigtbl]
              FROM results r
             WHERE r.view_schema = ANY($1)
            UNION ALL
            SELECT v.view_id, v.view_schema, v.view_name, v.view_column,
                   t.resorigtbl, t.resorigcol,
                   t.resorigtbl = ANY(v.path), v.path || t.resorigtbl
              FROM recursion v
              JOIN results t ON v.resorigtbl = t.view_id AND v.resorigcol = t.view_column
             WHERE NOT v.is_cycle
        )
        SELECT DISTINCT
               rec.view_schema, rec.view_name,
               sch.nspname AS base_schema, tbl.relname AS base_name,
               col.attname AS base_column, vcol.attname AS view_column
          FROM recursion rec
          JOIN pg_attribute vcol ON vcol.attrelid = rec.view_id
                                AND vcol.attnum = rec.view_column
                                AND NOT vcol.attisdropped
          JOIN pg_class tbl ON tbl.oid = rec.resorigtbl
          JOIN pg_namespace sch ON sch.oid = tbl.relnamespace
          JOIN pg_attribute col ON col.attrelid = tbl.oid
                               AND col.attnum = rec.resorigcol
                               AND NOT col.attisdropped
         WHERE rec.resorigtbl <> 0
        "#,
        &[],
        &[texts(schemas)],
    )
    .await?;

    rows.into_iter()
        .map(|row| -> Result<_> {
            Ok(ViewColumn {
                view: QualifiedIdentifier::new(
                    row.get::<String>("view_schema")?,
                    row.get::<String>("view_name")?,
                ),
                base: QualifiedIdentifier::new(
                    row.get::<String>("base_schema")?,
                    row.get::<String>("base_name")?,
                ),
                base_column: row.get::<String>("base_column")?,
                view_column: row.get::<String>("view_column")?,
            })
        })
        .collect()
}

/// Primary key columns for every table in the given schemas.
///
/// Junction detection needs the key of the table doing the joining, and that
/// table may sit in a schema which is not exposed -- the junction behind two
/// public views. Loading the key alone keeps it out of the API surface while
/// still letting the relationship be derived.
/// Every unique constraint, by the table it belongs to.
///
/// Returned as `(constraint name, columns)`. The primary key is one of them:
/// PostgreSQL treats it as a unique constraint and `ON CONFLICT ON CONSTRAINT`
/// accepts it, which is what an upsert on a table's key needs.
///
/// Only constraints, not bare unique indexes. `ON CONFLICT ON CONSTRAINT`
/// takes a constraint name, and a unique index created without one has no name
/// to give it.
pub async fn load_unique_constraints<T: Tx>(
    tx: &mut T,
    schemas: &[String],
) -> Result<HashMap<QualifiedIdentifier, Vec<(String, Vec<String>)>>> {
    let rows = catalog(
        tx,
        r#"
        SELECT
            n.nspname AS table_schema,
            c.relname AS table_name,
            con.conname AS constraint_name,
            (SELECT array_agg(a.attname ORDER BY k.ord)
               FROM unnest(con.conkey) WITH ORDINALITY AS k(attnum, ord)
               JOIN pg_attribute a
                 ON a.attrelid = c.oid AND a.attnum = k.attnum) AS columns
        FROM pg_constraint con
        JOIN pg_class c     ON c.oid = con.conrelid
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE con.contype IN ('p', 'u')
          AND n.nspname = ANY($1)
        "#,
        &[],
        &[texts(schemas)],
    )
    .await?;

    let mut by_table: HashMap<QualifiedIdentifier, Vec<(String, Vec<String>)>> = HashMap::new();
    for row in rows {
        let Some(columns): Option<Vec<String>> = row.get("columns")? else {
            continue;
        };
        by_table
            .entry(QualifiedIdentifier::new(
                row.get::<String>("table_schema")?,
                row.get::<String>("table_name")?,
            ))
            .or_default()
            .push((row.get::<String>("constraint_name")?, columns));
    }
    for constraints in by_table.values_mut() {
        constraints.sort();
    }
    Ok(by_table)
}

pub async fn load_primary_keys<T: Tx>(
    tx: &mut T,
    schemas: &[String],
) -> Result<HashMap<QualifiedIdentifier, Vec<String>>> {
    let rows = catalog(
        tx,
        r#"
        SELECT
            n.nspname AS table_schema,
            c.relname AS table_name,
            (SELECT array_agg(a.attname ORDER BY k.ord)
               FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord)
               JOIN pg_attribute a
                 ON a.attrelid = c.oid AND a.attnum = k.attnum) AS pk_cols
        FROM pg_index i
        JOIN pg_class c      ON c.oid = i.indrelid
        JOIN pg_namespace n  ON n.oid = c.relnamespace
        WHERE i.indisprimary
          AND n.nspname = ANY($1)
        "#,
        &[],
        &[texts(schemas)],
    )
    .await?;

    let mut keys = HashMap::new();
    for row in rows {
        let Some(columns): Option<Vec<String>> = row.get("pk_cols")? else {
            continue;
        };
        keys.insert(
            QualifiedIdentifier::new(
                row.get::<String>("table_schema")?,
                row.get::<String>("table_name")?,
            ),
            columns,
        );
    }
    Ok(keys)
}

/// The members of an enum table: its key column, and its `comment` column
/// when it has one, ordered by key.
///
/// Read as a role that may read the table. The connection reads it as itself
/// when it can, which a superuser or an inheriting authenticator always
/// could; a NOINHERIT authenticator can read nothing as itself, so it becomes
/// the first role it can become that holds SELECT on the table, inside a
/// transaction that is then rolled back.
#[cfg(feature = "sqlx")]
pub async fn enum_values(
    pool: &sqlx::PgPool,
    schema: &str,
    table: &str,
    key_column: &str,
    has_comment: bool,
) -> Result<Vec<(String, Option<String>)>> {
    let failed = |e: sqlx::Error| Error::SchemaCacheLoadFailed(e.to_string());
    let mut tx = pool.begin().await.map_err(failed)?;

    let reader: Option<String> = sqlx::query_scalar(
        r#"
        SELECT r.rolname::text
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
          JOIN pg_roles r ON pg_has_role(r.oid, 'MEMBER')
         WHERE n.nspname = $1 AND c.relname = $2
           AND has_schema_privilege(r.oid, n.oid, 'USAGE')
           AND has_table_privilege(r.oid, c.oid, 'SELECT')
         ORDER BY r.rolname = current_user DESC, r.rolname
         LIMIT 1
        "#,
    )
    .bind(schema)
    .bind(table)
    .fetch_optional(&mut *tx)
    .await
    .map_err(failed)?;
    let current: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut *tx)
        .await
        .map_err(failed)?;
    // No role can read it: read as the connection anyway, so the error says
    // what PostgreSQL says rather than something invented here.
    if let Some(reader) = reader.filter(|reader| *reader != current) {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "SET LOCAL ROLE {}",
            postrust_sql::escape_ident(&reader)
        )))
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    }

    let comment = match has_comment {
        true => format!("{}::text", postrust_sql::escape_ident("comment")),
        false => "NULL::text".to_string(),
    };
    let sql = format!(
        "SELECT {}::text, {} FROM {}.{} ORDER BY 1",
        postrust_sql::escape_ident(key_column),
        comment,
        postrust_sql::escape_ident(schema),
        postrust_sql::escape_ident(table)
    );
    let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .fetch_all(&mut *tx)
        .await
        .map_err(failed)?;
    tx.rollback().await.map_err(failed)?;

    Ok(rows
        .into_iter()
        .filter_map(|(value, comment)| value.map(|v| (v, comment)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(value: serde_json::Value) -> CatalogRow {
        match value {
            serde_json::Value::Object(map) => CatalogRow(map),
            other => panic!("not an object: {other}"),
        }
    }

    #[test]
    fn a_null_column_reads_as_none() {
        let row = row(json!({"description": null}));
        assert_eq!(row.get::<Option<String>>("description").unwrap(), None);
    }

    #[test]
    fn a_column_the_query_does_not_have_fails_the_load() {
        // A misspelled name: read as null, it would quietly be `None`.
        let row = row(json!({"description": "a table"}));
        let error = row.get::<Option<String>>("descripton").unwrap_err();
        assert!(
            matches!(&error, Error::SchemaCacheLoadFailed(m) if m.contains("descripton")),
            "{error:?}"
        );
    }

    #[test]
    fn a_value_of_the_wrong_type_fails_the_load() {
        let row = row(json!({"is_unique": "yes"}));
        assert!(matches!(
            row.get::<bool>("is_unique"),
            Err(Error::SchemaCacheLoadFailed(_))
        ));
    }

    #[test]
    fn the_wrapper_sorts_by_the_order_it_is_given() {
        assert_eq!(
            catalog_sql("SELECT 1 AS a, 2 AS b ORDER BY a", &["a", "b"]),
            "SELECT row_to_json(pgrst_catalog) FROM (SELECT 1 AS a, 2 AS b ORDER BY a) \
             AS pgrst_catalog ORDER BY pgrst_catalog.a, pgrst_catalog.b"
        );
        assert_eq!(
            catalog_sql("SELECT 1 AS a", &[]),
            "SELECT row_to_json(pgrst_catalog) FROM (SELECT 1 AS a) AS pgrst_catalog"
        );
    }
}

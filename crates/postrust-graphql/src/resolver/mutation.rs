//! Mutation resolvers for GraphQL insert/update/delete operations.
//!
//! Converts GraphQL mutation arguments into MutatePlan structures that can be executed.

use crate::input::mutation::InputValue;
use crate::resolver::query::TableFilter;
use bytes::Bytes;
use postrust_core::plan::{CoercibleField, CoercibleLogicTree, MutatePlan};
use postrust_core::schema_cache::Table;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Arguments for a GraphQL insert mutation.
#[derive(Debug, Clone, Default)]
pub struct InsertArgs {
    /// Objects to insert
    pub objects: Vec<HashMap<String, InputValue>>,
    /// On conflict handling
    pub on_conflict: Option<OnConflictArgs>,
    /// Fields to return
    pub returning: Vec<String>,
}

impl InsertArgs {
    /// Create new insert args.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an object to insert.
    pub fn with_object(mut self, object: HashMap<String, InputValue>) -> Self {
        self.objects.push(object);
        self
    }

    /// Add multiple objects to insert.
    pub fn with_objects(mut self, objects: Vec<HashMap<String, InputValue>>) -> Self {
        self.objects = objects;
        self
    }

    /// Set on conflict handling.
    pub fn with_on_conflict(mut self, on_conflict: OnConflictArgs) -> Self {
        self.on_conflict = Some(on_conflict);
        self
    }

    /// Set returning fields.
    pub fn with_returning(mut self, returning: Vec<String>) -> Self {
        self.returning = returning;
        self
    }

    /// Check if there are objects to insert.
    pub fn has_objects(&self) -> bool {
        !self.objects.is_empty()
    }

    /// Get the number of objects to insert.
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    /// Convert objects to JSON bytes.
    pub fn to_json_bytes(&self) -> Option<Bytes> {
        if self.objects.is_empty() {
            return None;
        }

        // Convert InputValue to serde_json::Value
        let json_objects: Vec<serde_json::Value> = self
            .objects
            .iter()
            .map(|obj| {
                let map: serde_json::Map<String, serde_json::Value> = obj
                    .iter()
                    .map(|(k, v)| (k.clone(), input_value_to_json(v)))
                    .collect();
                serde_json::Value::Object(map)
            })
            .collect();

        let json = if json_objects.len() == 1 {
            serde_json::to_vec(&json_objects[0]).ok()
        } else {
            serde_json::to_vec(&json_objects).ok()
        };

        json.map(Bytes::from)
    }

    /// Get column names from the first object.
    pub fn column_names(&self) -> Vec<String> {
        self.objects
            .first()
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// Arguments for on conflict handling.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OnConflictArgs {
    /// Constraint columns for conflict detection
    pub constraint: Vec<String>,
    /// Update action on conflict
    pub update_columns: Vec<String>,
    /// Additional where condition for update
    pub where_filter: Option<TableFilter>,
}

impl OnConflictArgs {
    /// Create new on conflict args.
    pub fn new(constraint: Vec<String>) -> Self {
        Self {
            constraint,
            update_columns: vec![],
            where_filter: None,
        }
    }

    /// Set columns to update on conflict.
    pub fn with_update_columns(mut self, columns: Vec<String>) -> Self {
        self.update_columns = columns;
        self
    }

    /// Set where filter for update.
    pub fn with_where(mut self, filter: TableFilter) -> Self {
        self.where_filter = Some(filter);
        self
    }
}

/// Arguments for a GraphQL update mutation.
#[derive(Debug, Clone, Default)]
pub struct UpdateArgs {
    /// Filter to select rows to update
    pub filter: Option<TableFilter>,
    /// Values to set
    pub set: HashMap<String, InputValue>,
    /// Fields to return
    pub returning: Vec<String>,
}

impl UpdateArgs {
    /// Create new update args.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the filter.
    pub fn with_filter(mut self, filter: TableFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Set the values to update.
    pub fn with_set(mut self, set: HashMap<String, InputValue>) -> Self {
        self.set = set;
        self
    }

    /// Set returning fields.
    pub fn with_returning(mut self, returning: Vec<String>) -> Self {
        self.returning = returning;
        self
    }

    /// Check if filter is specified.
    pub fn has_filter(&self) -> bool {
        self.filter.is_some()
    }

    /// Check if any values are set.
    pub fn has_set(&self) -> bool {
        !self.set.is_empty()
    }

    /// Convert set values to JSON bytes.
    pub fn to_json_bytes(&self) -> Option<Bytes> {
        if self.set.is_empty() {
            return None;
        }

        let map: serde_json::Map<String, serde_json::Value> = self
            .set
            .iter()
            .map(|(k, v)| (k.clone(), input_value_to_json(v)))
            .collect();

        serde_json::to_vec(&serde_json::Value::Object(map))
            .ok()
            .map(Bytes::from)
    }

    /// Get column names being updated.
    pub fn column_names(&self) -> Vec<String> {
        self.set.keys().cloned().collect()
    }
}

/// Arguments for a GraphQL delete mutation.
#[derive(Debug, Clone, Default)]
pub struct DeleteArgs {
    /// Filter to select rows to delete
    pub filter: Option<TableFilter>,
    /// Fields to return
    pub returning: Vec<String>,
}

impl DeleteArgs {
    /// Create new delete args.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the filter.
    pub fn with_filter(mut self, filter: TableFilter) -> Self {
        self.filter = Some(filter);
        self
    }

    /// Set returning fields.
    pub fn with_returning(mut self, returning: Vec<String>) -> Self {
        self.returning = returning;
        self
    }

    /// Check if filter is specified.
    pub fn has_filter(&self) -> bool {
        self.filter.is_some()
    }
}

/// Convert InputValue to serde_json::Value.
fn input_value_to_json(value: &InputValue) -> serde_json::Value {
    match value {
        InputValue::Null => serde_json::Value::Null,
        InputValue::Bool(b) => serde_json::Value::Bool(*b),
        InputValue::Int(i) => serde_json::Value::Number((*i).into()),
        InputValue::Float(f) => {
            serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        }
        InputValue::String(s) => serde_json::Value::String(s.clone()),
        InputValue::Object(obj) => {
            let map: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), input_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        InputValue::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(input_value_to_json).collect())
        }
    }
}

/// Build coercible fields from column names.
fn build_coercible_fields(columns: &[String], table: &Table) -> Vec<CoercibleField> {
    columns
        .iter()
        .filter_map(|name| {
            table
                .columns
                .get(name)
                .map(|col| CoercibleField::simple(name, &col.data_type))
        })
        .collect()
}

/// Build where clauses from a TableFilter.
fn build_where_clauses(filter: &Option<TableFilter>, table: &Table) -> Vec<CoercibleLogicTree> {
    let Some(filter) = filter else {
        return vec![];
    };

    let type_resolver = |name: &str| -> String {
        table
            .get_column(name)
            .map(|c| c.data_type.clone())
            .unwrap_or_else(|| "text".to_string())
    };

    filter
        .to_logic_tree()
        .map(|tree| vec![CoercibleLogicTree::from_logic_tree(&tree, type_resolver)])
        .unwrap_or_default()
}

/// Build an insert MutatePlan from GraphQL arguments.
pub fn build_insert_plan(args: &InsertArgs, table: &Table) -> MutatePlan {
    let columns = build_coercible_fields(&args.column_names(), table);
    let body = args.to_json_bytes();
    let returning = if args.returning.is_empty() {
        table.pk_cols.clone()
    } else {
        args.returning.clone()
    };

    let on_conflict = args.on_conflict.as_ref().map(|oc| {
        (
            postrust_core::api_request::PreferResolution::MergeDuplicates,
            oc.constraint.clone(),
        )
    });

    MutatePlan::Insert {
        target: table.qualified_identifier(),
        columns,
        body,
        on_conflict,
        where_clauses: vec![],
        returning,
        pk_cols: table.pk_cols.clone(),
        apply_defaults: true,
    }
}

/// Build an update MutatePlan from GraphQL arguments.
pub fn build_update_plan(args: &UpdateArgs, table: &Table) -> MutatePlan {
    let columns = build_coercible_fields(&args.column_names(), table);
    let body = args.to_json_bytes();
    let where_clauses = build_where_clauses(&args.filter, table);
    let returning = if args.returning.is_empty() {
        table.pk_cols.clone()
    } else {
        args.returning.clone()
    };

    MutatePlan::Update {
        target: table.qualified_identifier(),
        columns,
        body,
        where_clauses,
        returning,
        apply_defaults: false,
    }
}

/// Build a delete MutatePlan from GraphQL arguments.
pub fn build_delete_plan(args: &DeleteArgs, table: &Table) -> MutatePlan {
    let where_clauses = build_where_clauses(&args.filter, table);
    let returning = if args.returning.is_empty() {
        table.pk_cols.clone()
    } else {
        args.returning.clone()
    };

    MutatePlan::Delete {
        target: table.qualified_identifier(),
        where_clauses,
        returning,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::filter::IntFilterInput;
    use crate::resolver::query::FieldFilter;
    use indexmap::IndexMap;
    use postrust_core::schema_cache::Column;
    use pretty_assertions::assert_eq;

    fn create_test_table() -> Table {
        let mut columns = IndexMap::new();
        columns.insert(
            "id".into(),
            Column {
                name: "id".into(),
                description: None,
                nullable: false,
                data_type: "integer".into(),
                nominal_type: "int4".into(),
                max_len: None,
                default: Some("nextval('users_id_seq')".into()),
                enum_values: vec![],
                is_pk: true,
                position: 1,
            },
        );
        columns.insert(
            "name".into(),
            Column {
                name: "name".into(),
                description: None,
                nullable: false,
                data_type: "text".into(),
                nominal_type: "text".into(),
                max_len: None,
                default: None,
                enum_values: vec![],
                is_pk: false,
                position: 2,
            },
        );
        columns.insert(
            "email".into(),
            Column {
                name: "email".into(),
                description: None,
                nullable: true,
                data_type: "text".into(),
                nominal_type: "text".into(),
                max_len: None,
                default: None,
                enum_values: vec![],
                is_pk: false,
                position: 3,
            },
        );

        Table {
            schema: "public".into(),
            name: "users".into(),
            description: None,
            is_view: false,
            insertable: true,
            updatable: true,
            deletable: true,
            pk_cols: vec!["id".into()],
            columns,
        }
    }

    // ============================================================================
    // InsertArgs Tests
    // ============================================================================

    #[test]
    fn test_insert_args_default() {
        let args = InsertArgs::new();
        assert!(!args.has_objects());
        assert_eq!(args.object_count(), 0);
    }

    #[test]
    fn test_insert_args_with_object() {
        let mut object = HashMap::new();
        object.insert("name".to_string(), InputValue::String("Alice".to_string()));
        object.insert("email".to_string(), InputValue::String("alice@example.com".to_string()));

        let args = InsertArgs::new().with_object(object);
        assert!(args.has_objects());
        assert_eq!(args.object_count(), 1);
    }

    #[test]
    fn test_insert_args_with_multiple_objects() {
        let obj1: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Alice".to_string())),
        ].into_iter().collect();
        let obj2: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Bob".to_string())),
        ].into_iter().collect();

        let args = InsertArgs::new().with_objects(vec![obj1, obj2]);
        assert_eq!(args.object_count(), 2);
    }

    #[test]
    fn test_insert_args_column_names() {
        let object: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Alice".to_string())),
            ("email".to_string(), InputValue::String("alice@example.com".to_string())),
        ].into_iter().collect();

        let args = InsertArgs::new().with_object(object);
        let columns = args.column_names();
        assert_eq!(columns.len(), 2);
        assert!(columns.contains(&"name".to_string()));
        assert!(columns.contains(&"email".to_string()));
    }

    #[test]
    fn test_insert_args_to_json_bytes() {
        let object: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Alice".to_string())),
        ].into_iter().collect();

        let args = InsertArgs::new().with_object(object);
        let bytes = args.to_json_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["name"], "Alice");
    }

    #[test]
    fn test_insert_args_with_returning() {
        let args = InsertArgs::new()
            .with_returning(vec!["id".to_string(), "name".to_string()]);
        assert_eq!(args.returning.len(), 2);
    }

    #[test]
    fn test_insert_args_with_on_conflict() {
        let on_conflict = OnConflictArgs::new(vec!["email".to_string()])
            .with_update_columns(vec!["name".to_string()]);

        let args = InsertArgs::new().with_on_conflict(on_conflict);
        assert!(args.on_conflict.is_some());
    }

    // ============================================================================
    // OnConflictArgs Tests
    // ============================================================================

    #[test]
    fn test_on_conflict_args() {
        let args = OnConflictArgs::new(vec!["id".to_string()])
            .with_update_columns(vec!["name".to_string(), "email".to_string()]);

        assert_eq!(args.constraint, vec!["id".to_string()]);
        assert_eq!(args.update_columns.len(), 2);
    }

    // ============================================================================
    // UpdateArgs Tests
    // ============================================================================

    #[test]
    fn test_update_args_default() {
        let args = UpdateArgs::new();
        assert!(!args.has_filter());
        assert!(!args.has_set());
    }

    #[test]
    fn test_update_args_with_set() {
        let set: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Updated".to_string())),
        ].into_iter().collect();

        let args = UpdateArgs::new().with_set(set);
        assert!(args.has_set());
        assert_eq!(args.column_names().len(), 1);
    }

    #[test]
    fn test_update_args_with_filter() {
        let filter = TableFilter::new().with_field(
            "id",
            FieldFilter::int(IntFilterInput {
                eq: Some(1),
                ..Default::default()
            }),
        );

        let args = UpdateArgs::new().with_filter(filter);
        assert!(args.has_filter());
    }

    #[test]
    fn test_update_args_to_json_bytes() {
        let set: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Updated".to_string())),
            ("active".to_string(), InputValue::Bool(true)),
        ].into_iter().collect();

        let args = UpdateArgs::new().with_set(set);
        let bytes = args.to_json_bytes().unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["name"], "Updated");
        assert_eq!(json["active"], true);
    }

    // ============================================================================
    // DeleteArgs Tests
    // ============================================================================

    #[test]
    fn test_delete_args_default() {
        let args = DeleteArgs::new();
        assert!(!args.has_filter());
    }

    #[test]
    fn test_delete_args_with_filter() {
        let filter = TableFilter::new().with_field(
            "id",
            FieldFilter::int(IntFilterInput {
                eq: Some(1),
                ..Default::default()
            }),
        );

        let args = DeleteArgs::new().with_filter(filter);
        assert!(args.has_filter());
    }

    #[test]
    fn test_delete_args_with_returning() {
        let args = DeleteArgs::new()
            .with_returning(vec!["id".to_string(), "name".to_string()]);
        assert_eq!(args.returning.len(), 2);
    }

    // ============================================================================
    // InputValue to JSON Tests
    // ============================================================================

    #[test]
    fn test_input_value_to_json_null() {
        let json = input_value_to_json(&InputValue::Null);
        assert!(json.is_null());
    }

    #[test]
    fn test_input_value_to_json_bool() {
        let json = input_value_to_json(&InputValue::Bool(true));
        assert_eq!(json, serde_json::Value::Bool(true));
    }

    #[test]
    fn test_input_value_to_json_int() {
        let json = input_value_to_json(&InputValue::Int(42));
        assert_eq!(json, serde_json::json!(42));
    }

    #[test]
    fn test_input_value_to_json_float() {
        let json = input_value_to_json(&InputValue::Float(3.14));
        assert_eq!(json, serde_json::json!(3.14));
    }

    #[test]
    fn test_input_value_to_json_string() {
        let json = input_value_to_json(&InputValue::String("hello".to_string()));
        assert_eq!(json, serde_json::json!("hello"));
    }

    #[test]
    fn test_input_value_to_json_array() {
        let arr = vec![
            InputValue::Int(1),
            InputValue::Int(2),
            InputValue::Int(3),
        ];
        let json = input_value_to_json(&InputValue::Array(arr));
        assert_eq!(json, serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn test_input_value_to_json_object() {
        let obj: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("test".to_string())),
            ("count".to_string(), InputValue::Int(5)),
        ].into_iter().collect();
        let json = input_value_to_json(&InputValue::Object(obj));
        assert_eq!(json["name"], "test");
        assert_eq!(json["count"], 5);
    }

    // ============================================================================
    // MutatePlan Building Tests
    // ============================================================================

    #[test]
    fn test_build_insert_plan_basic() {
        let table = create_test_table();
        let object: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Alice".to_string())),
        ].into_iter().collect();

        let args = InsertArgs::new().with_object(object);
        let plan = build_insert_plan(&args, &table);

        match plan {
            MutatePlan::Insert { target, body, returning, .. } => {
                assert_eq!(target.name, "users");
                assert!(body.is_some());
                assert_eq!(returning, vec!["id".to_string()]);
            }
            _ => panic!("Expected Insert plan"),
        }
    }

    #[test]
    fn test_build_insert_plan_with_returning() {
        let table = create_test_table();
        let object: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Alice".to_string())),
        ].into_iter().collect();

        let args = InsertArgs::new()
            .with_object(object)
            .with_returning(vec!["id".to_string(), "name".to_string()]);
        let plan = build_insert_plan(&args, &table);

        match plan {
            MutatePlan::Insert { returning, .. } => {
                assert_eq!(returning.len(), 2);
            }
            _ => panic!("Expected Insert plan"),
        }
    }

    #[test]
    fn test_build_insert_plan_with_on_conflict() {
        let table = create_test_table();
        let object: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Alice".to_string())),
        ].into_iter().collect();

        let on_conflict = OnConflictArgs::new(vec!["id".to_string()]);
        let args = InsertArgs::new()
            .with_object(object)
            .with_on_conflict(on_conflict);
        let plan = build_insert_plan(&args, &table);

        match plan {
            MutatePlan::Insert { on_conflict, .. } => {
                assert!(on_conflict.is_some());
                let (_, cols) = on_conflict.unwrap();
                assert_eq!(cols, vec!["id".to_string()]);
            }
            _ => panic!("Expected Insert plan"),
        }
    }

    #[test]
    fn test_build_update_plan_basic() {
        let table = create_test_table();
        let set: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Updated".to_string())),
        ].into_iter().collect();

        let filter = TableFilter::new().with_field(
            "id",
            FieldFilter::int(IntFilterInput {
                eq: Some(1),
                ..Default::default()
            }),
        );

        let args = UpdateArgs::new()
            .with_set(set)
            .with_filter(filter);
        let plan = build_update_plan(&args, &table);

        match plan {
            MutatePlan::Update { target, body, where_clauses, .. } => {
                assert_eq!(target.name, "users");
                assert!(body.is_some());
                assert!(!where_clauses.is_empty());
            }
            _ => panic!("Expected Update plan"),
        }
    }

    #[test]
    fn test_build_update_plan_with_returning() {
        let table = create_test_table();
        let set: HashMap<String, InputValue> = [
            ("name".to_string(), InputValue::String("Updated".to_string())),
        ].into_iter().collect();

        let args = UpdateArgs::new()
            .with_set(set)
            .with_returning(vec!["id".to_string(), "name".to_string()]);
        let plan = build_update_plan(&args, &table);

        match plan {
            MutatePlan::Update { returning, .. } => {
                assert_eq!(returning.len(), 2);
            }
            _ => panic!("Expected Update plan"),
        }
    }

    #[test]
    fn test_build_delete_plan_basic() {
        let table = create_test_table();
        let filter = TableFilter::new().with_field(
            "id",
            FieldFilter::int(IntFilterInput {
                eq: Some(1),
                ..Default::default()
            }),
        );

        let args = DeleteArgs::new().with_filter(filter);
        let plan = build_delete_plan(&args, &table);

        match plan {
            MutatePlan::Delete { target, where_clauses, returning } => {
                assert_eq!(target.name, "users");
                assert!(!where_clauses.is_empty());
                assert_eq!(returning, vec!["id".to_string()]);
            }
            _ => panic!("Expected Delete plan"),
        }
    }

    #[test]
    fn test_build_delete_plan_with_returning() {
        let table = create_test_table();
        let args = DeleteArgs::new()
            .with_returning(vec!["id".to_string(), "name".to_string(), "email".to_string()]);
        let plan = build_delete_plan(&args, &table);

        match plan {
            MutatePlan::Delete { returning, .. } => {
                assert_eq!(returning.len(), 3);
            }
            _ => panic!("Expected Delete plan"),
        }
    }

    #[test]
    fn test_build_delete_plan_no_filter() {
        let table = create_test_table();
        let args = DeleteArgs::new();
        let plan = build_delete_plan(&args, &table);

        match plan {
            MutatePlan::Delete { where_clauses, .. } => {
                assert!(where_clauses.is_empty());
            }
            _ => panic!("Expected Delete plan"),
        }
    }
}

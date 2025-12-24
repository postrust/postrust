//! Order and pagination input types for GraphQL queries.
//!
//! Provides order by direction and pagination types for limiting and offsetting results.

use postrust_core::api_request::{Field, OrderDirection as CoreOrderDirection, OrderNulls, OrderTerm};
use serde::{Deserialize, Serialize};

/// Sort direction for ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderDirection {
    /// Ascending order (smallest first)
    Asc,
    /// Descending order (largest first)
    Desc,
}

impl Default for OrderDirection {
    fn default() -> Self {
        Self::Asc
    }
}

impl From<OrderDirection> for CoreOrderDirection {
    fn from(dir: OrderDirection) -> Self {
        match dir {
            OrderDirection::Asc => CoreOrderDirection::Asc,
            OrderDirection::Desc => CoreOrderDirection::Desc,
        }
    }
}

/// Null ordering preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NullsOrder {
    /// Nulls first
    First,
    /// Nulls last
    Last,
}

impl From<NullsOrder> for OrderNulls {
    fn from(nulls: NullsOrder) -> Self {
        match nulls {
            NullsOrder::First => OrderNulls::First,
            NullsOrder::Last => OrderNulls::Last,
        }
    }
}

/// An order by field specification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderByField {
    /// Field name to order by
    pub field: String,
    /// Direction of the sort
    pub direction: OrderDirection,
    /// Where to place nulls
    pub nulls: Option<NullsOrder>,
}

impl OrderByField {
    /// Create a new ascending order by field.
    pub fn asc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            direction: OrderDirection::Asc,
            nulls: None,
        }
    }

    /// Create a new descending order by field.
    pub fn desc(field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            direction: OrderDirection::Desc,
            nulls: None,
        }
    }

    /// Set nulls ordering.
    pub fn with_nulls(mut self, nulls: NullsOrder) -> Self {
        self.nulls = Some(nulls);
        self
    }

    /// Convert to an OrderTerm.
    pub fn to_order_term(&self) -> OrderTerm {
        OrderTerm::Field {
            field: Field::simple(&self.field),
            direction: Some(self.direction.into()),
            nulls: self.nulls.map(|n| n.into()),
        }
    }
}

/// Pagination input for limiting and offsetting results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaginationInput {
    /// Maximum number of results to return
    pub limit: Option<i64>,
    /// Number of results to skip
    pub offset: Option<i64>,
}

impl PaginationInput {
    /// Create pagination with a limit.
    pub fn new(limit: Option<i64>, offset: Option<i64>) -> Self {
        Self { limit, offset }
    }

    /// Create pagination with just a limit.
    pub fn with_limit(limit: i64) -> Self {
        Self {
            limit: Some(limit),
            offset: None,
        }
    }

    /// Create pagination with limit and offset.
    pub fn with_offset(limit: i64, offset: i64) -> Self {
        Self {
            limit: Some(limit),
            offset: Some(offset),
        }
    }

    /// Check if pagination is set.
    pub fn is_empty(&self) -> bool {
        self.limit.is_none() && self.offset.is_none()
    }

    /// Get the offset or 0 if not set.
    pub fn offset_or_default(&self) -> i64 {
        self.offset.unwrap_or(0)
    }
}

/// Combined order and pagination for a query.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrderAndPagination {
    /// Fields to order by
    pub order_by: Vec<OrderByField>,
    /// Pagination
    pub pagination: PaginationInput,
}

impl OrderAndPagination {
    /// Create new order and pagination settings.
    pub fn new(order_by: Vec<OrderByField>, pagination: PaginationInput) -> Self {
        Self {
            order_by,
            pagination,
        }
    }

    /// Convert order_by fields to OrderTerms.
    pub fn to_order_terms(&self) -> Vec<OrderTerm> {
        self.order_by.iter().map(|f| f.to_order_term()).collect()
    }
}

/// Helper to parse GraphQL order enum values like "id_ASC", "name_DESC".
pub fn parse_order_enum(value: &str) -> Option<OrderByField> {
    // Split by last underscore to handle field names with underscores
    if let Some(pos) = value.rfind('_') {
        let (field, direction) = value.split_at(pos);
        let direction = &direction[1..]; // Skip the underscore

        let dir = match direction {
            "ASC" => OrderDirection::Asc,
            "DESC" => OrderDirection::Desc,
            _ => return None,
        };

        Some(OrderByField {
            field: field.to_string(),
            direction: dir,
            nulls: None,
        })
    } else {
        None
    }
}

/// Generate order enum value from field and direction.
pub fn make_order_enum(field: &str, direction: OrderDirection) -> String {
    let dir_str = match direction {
        OrderDirection::Asc => "ASC",
        OrderDirection::Desc => "DESC",
    };
    format!("{}_{}", field, dir_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // ============================================================================
    // OrderDirection Tests
    // ============================================================================

    #[test]
    fn test_order_direction_default() {
        let dir = OrderDirection::default();
        assert_eq!(dir, OrderDirection::Asc);
    }

    #[test]
    fn test_order_direction_to_core() {
        let asc: CoreOrderDirection = OrderDirection::Asc.into();
        assert!(matches!(asc, CoreOrderDirection::Asc));

        let desc: CoreOrderDirection = OrderDirection::Desc.into();
        assert!(matches!(desc, CoreOrderDirection::Desc));
    }

    // ============================================================================
    // NullsOrder Tests
    // ============================================================================

    #[test]
    fn test_nulls_order_to_core() {
        let first: OrderNulls = NullsOrder::First.into();
        assert!(matches!(first, OrderNulls::First));

        let last: OrderNulls = NullsOrder::Last.into();
        assert!(matches!(last, OrderNulls::Last));
    }

    // ============================================================================
    // OrderByField Tests
    // ============================================================================

    #[test]
    fn test_order_by_field_asc() {
        let field = OrderByField::asc("name");
        assert_eq!(field.field, "name");
        assert_eq!(field.direction, OrderDirection::Asc);
        assert!(field.nulls.is_none());
    }

    #[test]
    fn test_order_by_field_desc() {
        let field = OrderByField::desc("created_at");
        assert_eq!(field.field, "created_at");
        assert_eq!(field.direction, OrderDirection::Desc);
    }

    #[test]
    fn test_order_by_field_with_nulls() {
        let field = OrderByField::desc("name").with_nulls(NullsOrder::Last);
        assert_eq!(field.nulls, Some(NullsOrder::Last));
    }

    #[test]
    fn test_order_by_field_to_order_term() {
        let field = OrderByField::desc("name").with_nulls(NullsOrder::First);
        let term = field.to_order_term();

        match term {
            OrderTerm::Field {
                field,
                direction,
                nulls,
            } => {
                assert_eq!(field.name, "name");
                assert!(matches!(direction, Some(CoreOrderDirection::Desc)));
                assert!(matches!(nulls, Some(OrderNulls::First)));
            }
            _ => panic!("Expected Field order term"),
        }
    }

    // ============================================================================
    // PaginationInput Tests
    // ============================================================================

    #[test]
    fn test_pagination_default() {
        let pagination = PaginationInput::default();
        assert!(pagination.limit.is_none());
        assert!(pagination.offset.is_none());
        assert!(pagination.is_empty());
    }

    #[test]
    fn test_pagination_with_limit() {
        let pagination = PaginationInput::with_limit(10);
        assert_eq!(pagination.limit, Some(10));
        assert!(pagination.offset.is_none());
        assert!(!pagination.is_empty());
    }

    #[test]
    fn test_pagination_with_offset() {
        let pagination = PaginationInput::with_offset(10, 20);
        assert_eq!(pagination.limit, Some(10));
        assert_eq!(pagination.offset, Some(20));
        assert!(!pagination.is_empty());
    }

    #[test]
    fn test_pagination_offset_or_default() {
        let pagination = PaginationInput::default();
        assert_eq!(pagination.offset_or_default(), 0);

        let pagination = PaginationInput::with_offset(10, 5);
        assert_eq!(pagination.offset_or_default(), 5);
    }

    // ============================================================================
    // OrderAndPagination Tests
    // ============================================================================

    #[test]
    fn test_order_and_pagination_default() {
        let oap = OrderAndPagination::default();
        assert!(oap.order_by.is_empty());
        assert!(oap.pagination.is_empty());
    }

    #[test]
    fn test_order_and_pagination_new() {
        let oap = OrderAndPagination::new(
            vec![OrderByField::desc("created_at")],
            PaginationInput::with_limit(10),
        );

        assert_eq!(oap.order_by.len(), 1);
        assert_eq!(oap.pagination.limit, Some(10));
    }

    #[test]
    fn test_order_and_pagination_to_order_terms() {
        let oap = OrderAndPagination::new(
            vec![
                OrderByField::desc("created_at"),
                OrderByField::asc("name"),
            ],
            PaginationInput::default(),
        );

        let terms = oap.to_order_terms();
        assert_eq!(terms.len(), 2);
    }

    // ============================================================================
    // Order Enum Parsing Tests
    // ============================================================================

    #[test]
    fn test_parse_order_enum_asc() {
        let field = parse_order_enum("name_ASC").unwrap();
        assert_eq!(field.field, "name");
        assert_eq!(field.direction, OrderDirection::Asc);
    }

    #[test]
    fn test_parse_order_enum_desc() {
        let field = parse_order_enum("created_at_DESC").unwrap();
        assert_eq!(field.field, "created_at");
        assert_eq!(field.direction, OrderDirection::Desc);
    }

    #[test]
    fn test_parse_order_enum_underscore_field() {
        let field = parse_order_enum("created_at_ASC").unwrap();
        assert_eq!(field.field, "created_at");
        assert_eq!(field.direction, OrderDirection::Asc);
    }

    #[test]
    fn test_parse_order_enum_invalid() {
        assert!(parse_order_enum("name").is_none());
        assert!(parse_order_enum("name_INVALID").is_none());
    }

    #[test]
    fn test_make_order_enum() {
        assert_eq!(make_order_enum("id", OrderDirection::Asc), "id_ASC");
        assert_eq!(make_order_enum("name", OrderDirection::Desc), "name_DESC");
        assert_eq!(
            make_order_enum("created_at", OrderDirection::Asc),
            "created_at_ASC"
        );
    }

    #[test]
    fn test_order_enum_roundtrip() {
        let original = OrderByField::desc("user_id");
        let enum_value = make_order_enum(&original.field, original.direction);
        let parsed = parse_order_enum(&enum_value).unwrap();

        assert_eq!(parsed.field, original.field);
        assert_eq!(parsed.direction, original.direction);
    }
}

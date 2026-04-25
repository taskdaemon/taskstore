// Query filtering for generic records

use crate::record::IndexValue;
use std::collections::HashMap;

/// Filter for querying records
#[derive(Debug, Clone)]
pub struct Filter {
    /// Field name to filter on
    pub field: String,
    /// Comparison operator
    pub op: FilterOp,
    /// Value to compare against
    pub value: IndexValue,
}

/// Comparison operators for filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    Eq,       // ==
    Ne,       // !=
    Gt,       // >
    Lt,       // <
    Gte,      // >=
    Lte,      // <=
    Contains, // LIKE %value%
}

impl FilterOp {
    #[allow(dead_code)]
    pub(crate) fn to_sql(self) -> &'static str {
        match self {
            FilterOp::Eq => "=",
            FilterOp::Ne => "!=",
            FilterOp::Gt => ">",
            FilterOp::Lt => "<",
            FilterOp::Gte => ">=",
            FilterOp::Lte => "<=",
            FilterOp::Contains => "LIKE",
        }
    }
}

/// In-Rust mirror of the SQL filter path used by `Store::list<T>`.
///
/// Returns `true` iff the record's indexed fields match the filter. Used by
/// `Store::list_tolerant<T>`, which bypasses SQLite and therefore cannot use
/// SQL filter pushdown.
///
/// Semantic carry-overs from the SQL path (`store.rs` list builder):
/// - Field absent from `fields` → `false` (mirrors SQL `EXISTS` returning empty).
/// - Filter value type does not match the indexed field's `IndexValue` variant
///   → `false` (mirrors SQL keying off the typed columns).
/// - `FilterOp::Contains` is ASCII-case-insensitive substring match (mirrors
///   SQLite's `LIKE` default for ASCII). Non-ASCII case folding intentionally
///   not handled - SQLite's `LIKE` does not handle it either.
pub(crate) fn match_filter(fields: &HashMap<String, IndexValue>, f: &Filter) -> bool {
    let field_value = match fields.get(&f.field) {
        Some(v) => v,
        None => return false,
    };
    match (field_value, &f.value) {
        (IndexValue::String(a), IndexValue::String(b)) => match f.op {
            FilterOp::Eq => a == b,
            FilterOp::Ne => a != b,
            FilterOp::Gt => a > b,
            FilterOp::Lt => a < b,
            FilterOp::Gte => a >= b,
            FilterOp::Lte => a <= b,
            FilterOp::Contains => a.to_ascii_lowercase().contains(&b.to_ascii_lowercase()),
        },
        (IndexValue::Int(a), IndexValue::Int(b)) => match f.op {
            FilterOp::Eq => a == b,
            FilterOp::Ne => a != b,
            FilterOp::Gt => a > b,
            FilterOp::Lt => a < b,
            FilterOp::Gte => a >= b,
            FilterOp::Lte => a <= b,
            FilterOp::Contains => false,
        },
        (IndexValue::Bool(a), IndexValue::Bool(b)) => match f.op {
            FilterOp::Eq => a == b,
            FilterOp::Ne => a != b,
            FilterOp::Gt => a > b,
            FilterOp::Lt => a < b,
            FilterOp::Gte => a >= b,
            FilterOp::Lte => a <= b,
            FilterOp::Contains => false,
        },
        // Type mismatch between indexed field and filter value
        _ => false,
    }
}

impl std::fmt::Display for FilterOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterOp::Eq => write!(f, "="),
            FilterOp::Ne => write!(f, "!="),
            FilterOp::Gt => write!(f, ">"),
            FilterOp::Lt => write!(f, "<"),
            FilterOp::Gte => write!(f, ">="),
            FilterOp::Lte => write!(f, "<="),
            FilterOp::Contains => write!(f, "LIKE"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_creation() {
        let filter = Filter {
            field: "status".to_string(),
            op: FilterOp::Eq,
            value: IndexValue::String("active".to_string()),
        };

        assert_eq!(filter.field, "status");
        assert_eq!(filter.op, FilterOp::Eq);
    }

    #[test]
    fn test_filter_op_to_sql() {
        assert_eq!(FilterOp::Eq.to_sql(), "=");
        assert_eq!(FilterOp::Ne.to_sql(), "!=");
        assert_eq!(FilterOp::Gt.to_sql(), ">");
        assert_eq!(FilterOp::Lt.to_sql(), "<");
        assert_eq!(FilterOp::Gte.to_sql(), ">=");
        assert_eq!(FilterOp::Lte.to_sql(), "<=");
        assert_eq!(FilterOp::Contains.to_sql(), "LIKE");
    }

    #[test]
    fn test_filter_op_display() {
        assert_eq!(FilterOp::Eq.to_string(), "=");
        assert_eq!(FilterOp::Ne.to_string(), "!=");
    }

    fn fields_str(field: &str, value: &str) -> HashMap<String, IndexValue> {
        let mut m = HashMap::new();
        m.insert(field.to_string(), IndexValue::String(value.to_string()));
        m
    }

    fn fields_int(field: &str, value: i64) -> HashMap<String, IndexValue> {
        let mut m = HashMap::new();
        m.insert(field.to_string(), IndexValue::Int(value));
        m
    }

    fn fields_bool(field: &str, value: bool) -> HashMap<String, IndexValue> {
        let mut m = HashMap::new();
        m.insert(field.to_string(), IndexValue::Bool(value));
        m
    }

    fn filter_str(field: &str, op: FilterOp, value: &str) -> Filter {
        Filter {
            field: field.to_string(),
            op,
            value: IndexValue::String(value.to_string()),
        }
    }

    fn filter_int(field: &str, op: FilterOp, value: i64) -> Filter {
        Filter {
            field: field.to_string(),
            op,
            value: IndexValue::Int(value),
        }
    }

    fn filter_bool(field: &str, op: FilterOp, value: bool) -> Filter {
        Filter {
            field: field.to_string(),
            op,
            value: IndexValue::Bool(value),
        }
    }

    #[test]
    fn match_filter_string_eq_ne() {
        let fields = fields_str("status", "active");
        assert!(match_filter(&fields, &filter_str("status", FilterOp::Eq, "active")));
        assert!(!match_filter(&fields, &filter_str("status", FilterOp::Eq, "draft")));
        assert!(match_filter(&fields, &filter_str("status", FilterOp::Ne, "draft")));
        assert!(!match_filter(&fields, &filter_str("status", FilterOp::Ne, "active")));
    }

    #[test]
    fn match_filter_string_ordering() {
        let fields = fields_str("name", "bravo");
        assert!(match_filter(&fields, &filter_str("name", FilterOp::Gt, "alpha")));
        assert!(match_filter(&fields, &filter_str("name", FilterOp::Lt, "charlie")));
        assert!(match_filter(&fields, &filter_str("name", FilterOp::Gte, "bravo")));
        assert!(match_filter(&fields, &filter_str("name", FilterOp::Lte, "bravo")));
        assert!(!match_filter(&fields, &filter_str("name", FilterOp::Gt, "bravo")));
    }

    #[test]
    fn match_filter_string_contains_case_insensitive() {
        let fields = fields_str("status", "Active");
        // ASCII-case-insensitive substring match (mirrors SQLite LIKE default).
        assert!(match_filter(
            &fields,
            &filter_str("status", FilterOp::Contains, "ACTIVE")
        ));
        assert!(match_filter(
            &fields,
            &filter_str("status", FilterOp::Contains, "activ")
        ));
        assert!(match_filter(&fields, &filter_str("status", FilterOp::Contains, "TIVE")));
        assert!(!match_filter(
            &fields,
            &filter_str("status", FilterOp::Contains, "draft")
        ));
    }

    #[test]
    fn match_filter_int_full() {
        let fields = fields_int("count", 42);
        assert!(match_filter(&fields, &filter_int("count", FilterOp::Eq, 42)));
        assert!(!match_filter(&fields, &filter_int("count", FilterOp::Eq, 43)));
        assert!(match_filter(&fields, &filter_int("count", FilterOp::Ne, 7)));
        assert!(match_filter(&fields, &filter_int("count", FilterOp::Gt, 41)));
        assert!(match_filter(&fields, &filter_int("count", FilterOp::Lt, 43)));
        assert!(match_filter(&fields, &filter_int("count", FilterOp::Gte, 42)));
        assert!(match_filter(&fields, &filter_int("count", FilterOp::Lte, 42)));
        assert!(!match_filter(&fields, &filter_int("count", FilterOp::Contains, 42)));
    }

    #[test]
    fn match_filter_bool_full() {
        let fields = fields_bool("active", true);
        assert!(match_filter(&fields, &filter_bool("active", FilterOp::Eq, true)));
        assert!(!match_filter(&fields, &filter_bool("active", FilterOp::Eq, false)));
        assert!(match_filter(&fields, &filter_bool("active", FilterOp::Ne, false)));
        assert!(match_filter(&fields, &filter_bool("active", FilterOp::Gt, false)));
        assert!(!match_filter(&fields, &filter_bool("active", FilterOp::Gt, true)));
        assert!(match_filter(&fields, &filter_bool("active", FilterOp::Gte, true)));
        assert!(!match_filter(&fields, &filter_bool("active", FilterOp::Contains, true)));
    }

    #[test]
    fn match_filter_missing_field_is_false() {
        // Mirrors SQL's EXISTS subquery returning empty when the indexed field is absent.
        let fields = fields_str("status", "active");
        assert!(!match_filter(&fields, &filter_str("missing", FilterOp::Eq, "x")));
    }

    #[test]
    fn match_filter_type_mismatch_is_false() {
        // Mirrors SQL keying off field_value_str / _int / _bool typed columns.
        let fields = fields_str("count", "forty-two");
        assert!(!match_filter(&fields, &filter_int("count", FilterOp::Eq, 42)));

        let fields = fields_int("status", 1);
        assert!(!match_filter(&fields, &filter_str("status", FilterOp::Eq, "1")));

        let fields = fields_bool("flag", true);
        assert!(!match_filter(&fields, &filter_int("flag", FilterOp::Eq, 1)));
    }
}

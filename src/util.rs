use chrono::{Datelike, NaiveDate, NaiveDateTime};
use serde::Serialize;
use sqlx::postgres::{PgTypeInfo, PgValueRef};
use sqlx::{Decode, Postgres, Type, ValueRef};

use crate::error::AppError;

/// Timestamp that decodes from either `TIMESTAMP` or `TIMESTAMPTZ` (the production
/// schema uses `TIMESTAMP`; a fresh migration may differ) and serializes as UTC
/// RFC3339 — matching node-postgres, which parses `timestamp` as UTC and emits `Z`.
#[derive(Debug, Clone, Copy)]
pub struct PgTimestamp(pub NaiveDateTime);

impl PgTimestamp {
    pub fn to_rfc3339(&self) -> String {
        self.0.and_utc().to_rfc3339()
    }
}

impl Type<Postgres> for PgTimestamp {
    fn type_info() -> PgTypeInfo {
        <NaiveDateTime as Type<Postgres>>::type_info()
    }
    fn compatible(ty: &PgTypeInfo) -> bool {
        *ty == <NaiveDateTime as Type<Postgres>>::type_info()
            || *ty == <chrono::DateTime<chrono::Utc> as Type<Postgres>>::type_info()
    }
}

impl<'r> Decode<'r, Postgres> for PgTimestamp {
    fn decode(value: PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        if *value.type_info() == <NaiveDateTime as Type<Postgres>>::type_info() {
            let dt = <NaiveDateTime as Decode<'r, Postgres>>::decode(value)?;
            Ok(PgTimestamp(dt))
        } else {
            let dt = <chrono::DateTime<chrono::Utc> as Decode<'r, Postgres>>::decode(value)?;
            Ok(PgTimestamp(dt.naive_utc()))
        }
    }
}

impl Serialize for PgTimestamp {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_rfc3339())
    }
}

/// Parse an optional "YYYY-MM-DD" string into a NaiveDate.
pub fn parse_date(s: Option<String>) -> Result<Option<NaiveDate>, AppError> {
    match s {
        Some(v) if !v.is_empty() => Ok(Some(
            NaiveDate::parse_from_str(&v, "%Y-%m-%d")
                .map_err(|_| AppError::BadRequest("Invalid date format".into()))?,
        )),
        _ => Ok(None),
    }
}

/// Port of `calculateMonthsDiff` in statsController.js: months since `start`,
/// including the current month (`+1`). Uses UTC date arithmetic.
pub fn calculate_months_diff(start: NaiveDate) -> i32 {
    calculate_months_diff_from(start, chrono::Utc::now().date_naive())
}

/// Testable core of `calculate_months_diff` with an explicit "now".
pub fn calculate_months_diff_from(start: NaiveDate, now: NaiveDate) -> i32 {
    (now.year() - start.year()) * 12 + (now.month0() as i32 - start.month0() as i32) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_diff_is_inclusive() {
        let start = NaiveDate::from_ymd_opt(2024, 8, 28).unwrap();
        let now = NaiveDate::from_ymd_opt(2026, 8, 4).unwrap();
        assert_eq!(calculate_months_diff_from(start, now), 25);
    }

    #[test]
    fn months_diff_same_month() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        assert_eq!(calculate_months_diff_from(d, d), 1);
    }

    #[test]
    fn parses_date_strings() {
        assert_eq!(
            parse_date(Some("2024-01-31".into())).unwrap(),
            Some(NaiveDate::from_ymd_opt(2024, 1, 31).unwrap())
        );
        assert_eq!(parse_date(None).unwrap(), None);
        assert_eq!(parse_date(Some(String::new())).unwrap(), None);
        assert!(parse_date(Some("not-a-date".into())).is_err());
    }
}

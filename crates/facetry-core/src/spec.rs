//! The declarative chart spec: what users write as JSON. Pure data, no behaviour beyond
//! parsing; [`crate::validate`] checks it against itself and [`crate::build_scene`] turns it
//! into geometry.
//!
//! Every struct rejects unknown fields so a typo (`"colour"`) is an error, not silence.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use serde::Deserialize;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
#[cfg(feature = "ts")]
use tsify::Tsify;

/// A named table: field name to column. All columns must have the same length.
pub type Table = BTreeMap<String, Column>;

/// The whole chart.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct ChartSpec {
    /// `"2d"` (the default). `"3d"` is reserved and rejected for now.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub dimension: Dimension,
    /// Columnar tables by name, e.g. `{ "sales": { "quarter": ["Q1"], "revenue": [120] } }`.
    /// A column is an array of numbers (`null` = missing) or an array of strings.
    #[serde(default)]
    #[cfg_attr(
        feature = "ts",
        tsify(
            optional,
            type = "Record<string, Record<string, (number | null)[] | string[]>>"
        )
    )]
    pub data: BTreeMap<String, Table>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub axes: Vec<AxisSpec>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub layers: Vec<LayerSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
pub enum Dimension {
    #[default]
    #[serde(rename = "2d")]
    TwoD,
    #[serde(rename = "3d")]
    ThreeD,
}

/// A data column. Strings are dictionary-encoded on the way in: each distinct string is
/// stored once, in order of first appearance, and rows hold a small index into that list.
#[derive(Debug, Clone, PartialEq)]
pub enum Column {
    /// `null` becomes NaN.
    Numeric(Vec<f64>),
    Categorical {
        codes: Vec<u32>,
        dict: Vec<String>,
    },
}

impl Column {
    pub fn len(&self) -> usize {
        match self {
            Self::Numeric(v) => v.len(),
            Self::Categorical { codes, .. } => codes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `"numeric"` or `"categorical"`, for error messages.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Numeric(_) => "numeric",
            Self::Categorical { .. } => "categorical",
        }
    }
}

/// One JSON array element.
enum Cell {
    Number(f64),
    Text(String),
    Null,
}

impl<'de> Deserialize<'de> for Cell {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct CellVisitor;
        impl Visitor<'_> for CellVisitor {
            type Value = Cell;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a number, a string or null")
            }
            fn visit_f64<E>(self, v: f64) -> Result<Cell, E> {
                Ok(Cell::Number(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Cell, E> {
                Ok(Cell::Number(v as f64))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Cell, E> {
                Ok(Cell::Number(v as f64))
            }
            fn visit_str<E>(self, v: &str) -> Result<Cell, E> {
                Ok(Cell::Text(v.to_owned()))
            }
            fn visit_unit<E>(self) -> Result<Cell, E> {
                Ok(Cell::Null)
            }
            fn visit_none<E>(self) -> Result<Cell, E> {
                Ok(Cell::Null)
            }
        }
        d.deserialize_any(CellVisitor)
    }
}

impl<'de> Deserialize<'de> for Column {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ColumnVisitor;
        impl<'de> Visitor<'de> for ColumnVisitor {
            type Value = Column;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an array of numbers or an array of strings")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Column, A::Error> {
                let mut numbers = Vec::new();
                let mut texts: Vec<String> = Vec::new();
                let mut len = 0usize;
                while let Some(cell) = seq.next_element::<Cell>()? {
                    len += 1;
                    match cell {
                        Cell::Number(v) => numbers.push(v),
                        Cell::Null => numbers.push(f64::NAN),
                        Cell::Text(s) => texts.push(s),
                    }
                }
                if texts.is_empty() {
                    return Ok(Column::Numeric(numbers));
                }
                // Anything that is not a string (numbers, nulls) makes the column ambiguous.
                if texts.len() != len {
                    return Err(de::Error::custom(
                        "a column is either all numbers (null allowed) or all strings, not a mix",
                    ));
                }
                let mut index: HashMap<&str, u32> = HashMap::new();
                let mut dict: Vec<&str> = Vec::new();
                let codes = texts
                    .iter()
                    .map(|s| {
                        *index.entry(s).or_insert_with(|| {
                            dict.push(s);
                            (dict.len() - 1) as u32
                        })
                    })
                    .collect();
                let dict = dict.into_iter().map(str::to_owned).collect();
                Ok(Column::Categorical { codes, dict })
            }
        }
        d.deserialize_seq(ColumnVisitor)
    }
}

/// Where an axis sits. `Depth` belongs to 3D charts and is rejected for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
pub enum Orient {
    Bottom,
    Left,
    Right,
    Depth,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct AxisSpec {
    /// Referenced by `encoding.*.axis`.
    pub id: String,
    pub orient: Orient,
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub title: Option<String>,
    /// Overrides for a numeric (linear) axis.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub scale: Option<ScaleSpec>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct ScaleSpec {
    /// Fixed `[min, max]`; by default the domain is computed from the data.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub domain: Option<[f64; 2]>,
    /// Round the domain out to tick values. Default: `true`, unless `domain` is given.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub nice: Option<bool>,
    /// Make the computed domain include 0. Default: `true` (bars grow from zero).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub zero: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(rename_all = "lowercase")]
pub enum Mark {
    Bar,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    pub mark: Mark,
    /// Name of a table in `data`.
    pub data: String,
    pub encoding: Encoding,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct Encoding {
    /// Must be a categorical field (band scale).
    pub x: PositionEncoding,
    /// Must be a numeric field (linear scale).
    pub y: PositionEncoding,
    /// A fixed colour, or a categorical field mapped onto the theme palette.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub color: Option<ColorEncoding>,
    /// Stable identity of each mark across updates. Default: the `x` value.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub key: Option<KeyEncoding>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct PositionEncoding {
    pub field: String,
    /// Id of the axis to bind to. Default: the first declared axis that fits the channel
    /// (`bottom` for x, `left` or `right` for y).
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub axis: Option<String>,
}

/// Exactly one of `value` and `field`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct ColorEncoding {
    /// A CSS hex colour, `#rrggbb` or `#rrggbbaa`.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub value: Option<String>,
    /// A categorical field: each distinct value gets the next palette colour.
    #[serde(default)]
    #[cfg_attr(feature = "ts", tsify(optional))]
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[cfg_attr(feature = "ts", derive(Tsify))]
#[serde(deny_unknown_fields)]
pub struct KeyEncoding {
    pub field: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(json: &str) -> Result<Column, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn numeric_columns_turn_null_into_nan() {
        let Column::Numeric(v) = column("[1, null, 2.5]").unwrap() else {
            panic!("expected numeric");
        };
        assert_eq!(v[0], 1.0);
        assert!(v[1].is_nan());
        assert_eq!(v[2], 2.5);
    }

    #[test]
    fn string_columns_are_dictionary_encoded_in_first_seen_order() {
        let c = column(r#"["b", "a", "b", "c", "a"]"#).unwrap();
        assert_eq!(
            c,
            Column::Categorical {
                codes: vec![0, 1, 0, 2, 1],
                dict: vec!["b".into(), "a".into(), "c".into()],
            }
        );
    }

    #[test]
    fn empty_column_is_numeric_and_empty() {
        let c = column("[]").unwrap();
        assert!(c.is_empty());
        assert_eq!(c.kind(), "numeric");
    }

    #[test]
    fn mixed_and_exotic_columns_are_rejected() {
        assert!(column(r#"["a", 1]"#).is_err());
        assert!(column(r#"["a", null]"#).is_err());
        assert!(column("[true]").is_err());
        assert!(column("5").is_err());
    }
}

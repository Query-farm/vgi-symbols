//! Small Arrow column-reading helpers shared by the scalar / table functions.

use arrow_array::cast::AsArray;
use arrow_array::types::{Int32Type, Int64Type, UInt32Type, UInt64Type};
use arrow_array::{Array, ArrayRef};
use arrow_schema::DataType;
use vgi::arguments::Arguments;

/// Read a const BLOB argument at `pos` as owned bytes (row 0), accepting BLOB or
/// VARCHAR-as-bytes. `None` if absent / null / wrong type.
pub fn const_blob(args: &Arguments, pos: usize) -> Option<Vec<u8>> {
    let arr = args.arg(pos)?;
    if arr.is_empty() || arr.is_null(0) {
        return None;
    }
    match arr.data_type() {
        DataType::Binary => Some(arr.as_binary::<i32>().value(0).to_vec()),
        DataType::LargeBinary => Some(arr.as_binary::<i64>().value(0).to_vec()),
        DataType::Utf8 => Some(arr.as_string::<i32>().value(0).as_bytes().to_vec()),
        DataType::LargeUtf8 => Some(arr.as_string::<i64>().value(0).as_bytes().to_vec()),
        _ => None,
    }
}

/// Read a build-id string cell (VARCHAR), or `None` if null / not a string.
pub fn build_id_at(col: &ArrayRef, i: usize) -> Option<String> {
    if col.is_null(i) {
        return None;
    }
    match col.data_type() {
        DataType::Utf8 => Some(col.as_string::<i32>().value(i).to_string()),
        DataType::LargeUtf8 => Some(col.as_string::<i64>().value(i).to_string()),
        _ => None,
    }
}

/// Read an address cell as `u64`, accepting any integer width (UBIGINT is the
/// declared type, but a literal may bind narrower / signed).
pub fn address_at(col: &ArrayRef, i: usize) -> Option<u64> {
    if col.is_null(i) {
        return None;
    }
    match col.data_type() {
        DataType::UInt64 => Some(col.as_primitive::<UInt64Type>().value(i)),
        DataType::Int64 => Some(col.as_primitive::<Int64Type>().value(i) as u64),
        DataType::UInt32 => Some(col.as_primitive::<UInt32Type>().value(i) as u64),
        DataType::Int32 => Some(col.as_primitive::<Int32Type>().value(i) as u64),
        _ => None,
    }
}

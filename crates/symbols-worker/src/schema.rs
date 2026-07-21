//! Arrow shapes for the symbolication surface: the STRUCT / LIST shapes for the
//! `symbolicate` and `inline_frames` scalars.

use std::sync::Arc;

use arrow_array::builder::{Int32Builder, ListBuilder, StringBuilder, StructBuilder};
use arrow_array::{ArrayRef, StructArray};
use arrow_schema::{DataType, Field, Fields};

/// The fields of one inline sub-frame `STRUCT(function, file, line)`.
pub fn inline_struct_fields() -> Fields {
    Fields::from(vec![
        Field::new("function", DataType::Utf8, true),
        Field::new("file", DataType::Utf8, true),
        Field::new("line", DataType::Int32, true),
    ])
}

/// The `symbolicate` scalar return STRUCT type.
pub fn symbolicate_struct_type() -> DataType {
    DataType::Struct(Fields::from(vec![
        Field::new("function", DataType::Utf8, true),
        Field::new("file", DataType::Utf8, true),
        Field::new("line", DataType::Int32, true),
        Field::new(
            "inlined",
            DataType::List(Arc::new(Field::new(
                "item",
                DataType::Struct(inline_struct_fields()),
                true,
            ))),
            true,
        ),
        Field::new("module", DataType::Utf8, true),
        Field::new("debug_id", DataType::Utf8, true),
        Field::new("status", DataType::Utf8, false),
    ]))
}

/// The `inline_frames` scalar return type: `LIST<STRUCT(function, file, line)>`.
pub fn inline_frames_list_type() -> DataType {
    DataType::List(Arc::new(Field::new(
        "item",
        DataType::Struct(inline_struct_fields()),
        true,
    )))
}

/// One inline sub-frame as plain data: (function, file, line).
pub type InlineCell = (Option<String>, Option<String>, Option<i32>);

/// Build a `LIST<STRUCT(function,file,line)>` array from per-row inline chains
/// (innermost-first, physical frame excluded). One list per input row.
pub fn build_inline_list(per_row: &[Vec<InlineCell>]) -> ArrayRef {
    let struct_fields = vec![
        Field::new("function", DataType::Utf8, true),
        Field::new("file", DataType::Utf8, true),
        Field::new("line", DataType::Int32, true),
    ];
    let builder = StructBuilder::from_fields(struct_fields, 0);
    let mut list = ListBuilder::new(builder);
    for chain in per_row {
        let sb = list.values();
        for (func, file, line) in chain {
            sb.field_builder::<StringBuilder>(0)
                .unwrap()
                .append_option(func.as_deref());
            sb.field_builder::<StringBuilder>(1)
                .unwrap()
                .append_option(file.as_deref());
            sb.field_builder::<Int32Builder>(2)
                .unwrap()
                .append_option(*line);
            sb.append(true);
        }
        list.append(true);
    }
    Arc::new(list.finish())
}

/// Assemble a single-column `StructArray` from named child arrays.
pub fn struct_array(fields: Vec<(Field, ArrayRef)>) -> StructArray {
    let pairs: Vec<(Arc<Field>, ArrayRef)> =
        fields.into_iter().map(|(f, a)| (Arc::new(f), a)).collect();
    StructArray::from(pairs)
}

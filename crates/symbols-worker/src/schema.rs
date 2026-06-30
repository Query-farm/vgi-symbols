//! Arrow shapes for the symbolication surface: the inline-expanded resolved-row
//! table schema (shared by `resolve` / `resolve_batch`), plus the STRUCT / LIST
//! shapes for the `symbolicate` and `inline_frames` scalars.

use std::sync::Arc;

use arrow_array::builder::{
    BooleanBuilder, Int32Builder, ListBuilder, StringBuilder, StructBuilder, UInt64Builder,
};
use arrow_array::{ArrayRef, RecordBatch, StructArray};
use arrow_schema::{DataType, Field, Fields, Schema, SchemaRef};
use symbols_core::frame::ResolvedFrame;
use vgi_rpc::{Result, RpcError};

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

/// The inline-expanded resolved-row schema. When `with_frame_idx` is set the
/// leading `frame_idx` column (the index back into a `resolve_batch` input list)
/// is present.
pub fn resolved_schema(with_frame_idx: bool) -> SchemaRef {
    let mut fields = Vec::new();
    if with_frame_idx {
        fields.push(Field::new("frame_idx", DataType::Int32, false));
    }
    fields.extend([
        Field::new("build_id", DataType::Utf8, true),
        Field::new("address", DataType::UInt64, true),
        Field::new("inline_depth", DataType::Int32, false),
        Field::new("is_inline", DataType::Boolean, false),
        Field::new("function", DataType::Utf8, true),
        Field::new("function_raw", DataType::Utf8, true),
        Field::new("file", DataType::Utf8, true),
        Field::new("line", DataType::Int32, true),
        Field::new("column", DataType::Int32, true),
        Field::new("module", DataType::Utf8, true),
        Field::new("debug_id", DataType::Utf8, true),
        Field::new("status", DataType::Utf8, false),
    ]);
    Arc::new(Schema::new(fields))
}

/// One echoed input frame plus the resolver's rows for it.
pub struct ResolvedRows {
    /// `frame_idx` (only used by `resolve_batch`).
    pub frame_idx: Option<i32>,
    /// Echoed input build-id.
    pub build_id: String,
    /// Echoed input address.
    pub address: u64,
    /// The inline-expanded frames for this address.
    pub frames: Vec<ResolvedFrame>,
}

/// Accumulating builder for the resolved-row table.
pub struct ResolvedBatchBuilder {
    schema: SchemaRef,
    with_frame_idx: bool,
    frame_idx: Int32Builder,
    build_id: StringBuilder,
    address: UInt64Builder,
    inline_depth: Int32Builder,
    is_inline: BooleanBuilder,
    function: StringBuilder,
    function_raw: StringBuilder,
    file: StringBuilder,
    line: Int32Builder,
    column: Int32Builder,
    module: StringBuilder,
    debug_id: StringBuilder,
    status: StringBuilder,
    rows: usize,
}

impl ResolvedBatchBuilder {
    /// A fresh builder for the given output schema (which may be projection-
    /// narrowed; we still build the full set of columns and project at the end).
    pub fn new(with_frame_idx: bool) -> ResolvedBatchBuilder {
        ResolvedBatchBuilder {
            schema: resolved_schema(with_frame_idx),
            with_frame_idx,
            frame_idx: Int32Builder::new(),
            build_id: StringBuilder::new(),
            address: UInt64Builder::new(),
            inline_depth: Int32Builder::new(),
            is_inline: BooleanBuilder::new(),
            function: StringBuilder::new(),
            function_raw: StringBuilder::new(),
            file: StringBuilder::new(),
            line: Int32Builder::new(),
            column: Int32Builder::new(),
            module: StringBuilder::new(),
            debug_id: StringBuilder::new(),
            status: StringBuilder::new(),
            rows: 0,
        }
    }

    /// Append every inline-expanded row for one echoed input frame.
    pub fn push(&mut self, row: &ResolvedRows) {
        for f in &row.frames {
            if self.with_frame_idx {
                self.frame_idx.append_value(row.frame_idx.unwrap_or(0));
            }
            self.build_id.append_value(&row.build_id);
            self.address.append_value(row.address);
            self.inline_depth.append_value(f.inline_depth);
            self.is_inline.append_value(f.is_inline);
            append_opt(&mut self.function, f.function.as_deref());
            append_opt(&mut self.function_raw, f.function_raw.as_deref());
            append_opt(&mut self.file, f.file.as_deref());
            append_opt_i32(&mut self.line, f.line.map(|l| l as i32));
            append_opt_i32(&mut self.column, f.column.map(|c| c as i32));
            append_opt(&mut self.module, f.module.as_deref());
            append_opt(&mut self.debug_id, f.debug_id.as_deref());
            self.status.append_value(f.status.as_status());
            self.rows += 1;
        }
    }

    /// Finish into a `RecordBatch` with the full resolved schema.
    pub fn finish(mut self) -> Result<RecordBatch> {
        let mut cols: Vec<ArrayRef> = Vec::new();
        if self.with_frame_idx {
            cols.push(Arc::new(self.frame_idx.finish()));
        }
        cols.push(Arc::new(self.build_id.finish()));
        cols.push(Arc::new(self.address.finish()));
        cols.push(Arc::new(self.inline_depth.finish()));
        cols.push(Arc::new(self.is_inline.finish()));
        cols.push(Arc::new(self.function.finish()));
        cols.push(Arc::new(self.function_raw.finish()));
        cols.push(Arc::new(self.file.finish()));
        cols.push(Arc::new(self.line.finish()));
        cols.push(Arc::new(self.column.finish()));
        cols.push(Arc::new(self.module.finish()));
        cols.push(Arc::new(self.debug_id.finish()));
        cols.push(Arc::new(self.status.finish()));
        RecordBatch::try_new(self.schema.clone(), cols)
            .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

fn append_opt(b: &mut StringBuilder, v: Option<&str>) {
    match v {
        Some(s) => b.append_value(s),
        None => b.append_null(),
    }
}

fn append_opt_i32(b: &mut Int32Builder, v: Option<i32>) {
    match v {
        Some(n) => b.append_value(n),
        None => b.append_null(),
    }
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

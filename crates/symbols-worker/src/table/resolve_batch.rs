//! `resolve_batch(frames LIST<STRUCT(build_id, address)>) -> TABLE(frame_idx, …)`
//! — one call, a whole list of frames (e.g. `list(...)` over a pprof location
//! table). `frame_idx` indexes back into the input list. The list is grouped by
//! debug-id internally (via the shared cache) so each module is touched once.

use arrow_array::cast::AsArray;
use arrow_array::{Array, RecordBatch, StructArray};
use arrow_schema::DataType;
use vgi::table_in_out::TableInOutFunction;
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::{Result, RpcError};

use crate::schema::{resolved_schema, ResolvedBatchBuilder, ResolvedRows};
use crate::state::with_state;
use crate::util::{address_at, build_id_at};

/// `resolve_batch`.
pub struct ResolveBatch;

impl TableInOutFunction for ResolveBatch {
    fn name(&self) -> &str {
        "resolve_batch"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Resolve a Frame List (batch)",
            "Resolve a whole LIST of `(build_id, address)` frames in one call — e.g. `list({build_id, \
             address})` aggregated over a pprof / minidump location table. Returns the same \
             inline-expanded rows as `resolve`, prefixed with `frame_idx` (the index back into the \
             input list). Frames are grouped by debug-id internally so each debug module is parsed \
             once per call (the vectorized fast path). An unresolved frame still yields exactly one \
             `status='not_found'` row — the result is total. Addresses are MODULE-RELATIVE (the \
             caller subtracts the load base).",
            "Resolve a whole `LIST<STRUCT(build_id, address)>` in one pass: \
             `SELECT * FROM symbols.resolve_batch((SELECT list({build_id, address}) FROM locs))`. \
             Rows carry `frame_idx` back into the list. Addresses are module-relative.",
            "resolve_batch, batch, list, pprof, minidump, perf, vectorized, symbolicate, inline, \
             frame_idx, build_id, address",
        );
        tags.push(("vgi.result_columns_md".into(), RESULT_COLUMNS_MD.into()));
        FunctionMetadata {
            description: "Resolve a LIST of (build_id, address) frames in one pass; rows carry \
                          frame_idx back into the input list"
                .into(),
            examples: vec![FunctionExample {
                sql: "SELECT * FROM symbols.resolve_batch((SELECT list({build_id: build_id, \
                      address: addr}) FROM pprof_locations));"
                    .into(),
                description: "Symbolicate a whole pprof location table in one pass.".into(),
                expected_output: None,
            }],
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![ArgSpec::any_column(
            "frames",
            0,
            "A LIST<STRUCT(build_id VARCHAR, address UBIGINT)> of frames to resolve, e.g. \
             `list({build_id: build_id, address: addr})` over a location table. `address` is the \
             MODULE-RELATIVE virtual address (caller subtracts the load base).",
        )]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: resolved_schema(true),
            opaque_data: Vec::new(),
        })
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<Vec<RecordBatch>> {
        // The frame list arrives as the input batch's first column (DuckDB
        // materializes the LIST-valued subquery argument into the input stream).
        // With no input rows there is nothing to do — return empty rather than a
        // row per empty tick, which would loop the exchange.
        if batch.num_columns() < 1 || batch.num_rows() == 0 {
            return Ok(vec![ResolvedBatchBuilder::new(true).finish()?]);
        }
        let col = batch.column(0);
        let list = col
            .as_list_opt::<i32>()
            .ok_or_else(|| RpcError::type_error("resolve_batch: argument must be a LIST"))?;

        let mut builder = ResolvedBatchBuilder::new(true);
        with_state(|state| -> Result<()> {
            for r in 0..list.len() {
                if list.is_null(r) {
                    continue;
                }
                let elems = list.value(r);
                let st = elems
                    .as_any()
                    .downcast_ref::<StructArray>()
                    .ok_or_else(|| {
                        RpcError::type_error("resolve_batch: list elements must be STRUCT")
                    })?;
                let build = column_by_name(st, "build_id")?;
                let addr = column_by_name(st, "address")?;
                for j in 0..st.len() {
                    let b = build_id_at(&build, j);
                    let a = address_at(&addr, j);
                    let frames = match (b.as_deref(), a) {
                        (Some(b), Some(a)) => state.resolve(b, a),
                        _ => vec![symbols_core::frame::ResolvedFrame::not_found()],
                    };
                    builder.push(&ResolvedRows {
                        frame_idx: Some(j as i32),
                        build_id: b.unwrap_or_default(),
                        address: a.unwrap_or(0),
                        frames,
                    });
                }
            }
            Ok(())
        })?;
        let out = builder.finish()?;
        Ok(vec![vgi::table_in_out::project_batch(
            &out,
            &params.output_schema,
        )?])
    }
}

/// Fetch a struct field array by name (the struct may name fields build_id/address).
fn column_by_name(st: &StructArray, name: &str) -> Result<arrow_array::ArrayRef> {
    if let Some((idx, _)) = st
        .fields()
        .iter()
        .enumerate()
        .find(|(_, f)| f.name() == name)
    {
        return Ok(st.column(idx).clone());
    }
    // Fall back to positional: build_id first, address second.
    let pos = if name == "build_id" { 0 } else { 1 };
    if st.num_columns() > pos {
        // Sanity: only accept positional fallback when the type is plausible.
        let ty = st.column(pos).data_type();
        let ok = match name {
            "build_id" => matches!(ty, DataType::Utf8 | DataType::LargeUtf8),
            _ => ty.is_integer(),
        };
        if ok {
            return Ok(st.column(pos).clone());
        }
    }
    Err(RpcError::value_error(format!(
        "resolve_batch: list STRUCT is missing a '{name}' field"
    )))
}

const RESULT_COLUMNS_MD: &str = "Same columns as `resolve`, prefixed with `frame_idx`:\n\n\
| column | type | description |\n\
|---|---|---|\n\
| `frame_idx` | INTEGER | Index back into the input frame list. |\n\
| `build_id` | VARCHAR | Echoed input build-id. |\n\
| `address` | UBIGINT | Echoed input address. |\n\
| `inline_depth` | INTEGER | 0 = innermost inlined … N = physical frame. |\n\
| `is_inline` | BOOLEAN | True for inline frames, false for the physical frame. |\n\
| `function` / `function_raw` | VARCHAR | Demangled / raw name. |\n\
| `file` / `line` / `column` | VARCHAR/INTEGER/INTEGER | Source location. |\n\
| `module` / `debug_id` | VARCHAR | Debug file + normalized key that answered. |\n\
| `status` | VARCHAR | 'ok' / 'not_found' / 'no_line' / 'error:<kind>'. |";

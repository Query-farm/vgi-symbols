//! `symbolicate(build_id, address) -> STRUCT(...)` — the scalar convenience: the
//! physical frame's symbol with the inline chain collapsed into the `inlined`
//! LIST (innermost-first). Same resolution as `resolve`, one row out.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field};
use symbols_core::frame::ResolvedFrame;
use vgi::{
    ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams,
    ScalarFunction,
};
use vgi_rpc::{Result, RpcError};

use crate::schema::{
    build_inline_list, inline_frames_list_type, struct_array, symbolicate_struct_type,
};
use crate::state::with_state;
use crate::util::{address_at, build_id_at};

/// `symbolicate`.
pub struct Symbolicate;

impl ScalarFunction for Symbolicate {
    fn name(&self) -> &str {
        "symbolicate"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Symbolicate Frame (scalar)",
            "Resolve a single `(build_id, address)` stack frame to a STRUCT with the physical \
             frame's `function` / `file` / `line`, the inlined call chain collapsed into an \
             `inlined` LIST<STRUCT(function, file, line)> (innermost-first), and the `module`, \
             `debug_id`, and `status` that answered. Same resolution as the `resolve` table \
             function, but one row out — the convenience for resolving one frame in a scalar \
             context. `build_id` is the normalized debug-id or raw build-id hex; `address` is the \
             MODULE-RELATIVE virtual address (caller subtracts the load base). `status` is 'ok', \
             'not_found' (no module), 'no_line' (no line info), or 'error:<kind>'. Backed by the \
             persistent build-id-keyed cache.",
            "Resolve one `(build_id, address)` to a STRUCT(function, file, line, inlined, module, \
             debug_id, status); `inlined` is the innermost-first inline chain. `address` is \
             module-relative. Use `resolve` for the inline-expanded table form.",
            "symbolicate, resolve, stack frame, function, file, line, inline, addr2line, dwarf, \
             pdb, build_id, address, crash, profiling",
        );
        tags.push((
            "vgi.result_columns_md".into(),
            "Returns a single STRUCT column:\n\n\
             | field | type | description |\n\
             |---|---|---|\n\
             | `function` | VARCHAR | Physical frame's demangled function name. |\n\
             | `file` | VARCHAR | Source file. |\n\
             | `line` | INTEGER | 1-based source line. |\n\
             | `inlined` | LIST<STRUCT(function,file,line)> | Inline chain, innermost-first. |\n\
             | `module` | VARCHAR | Debug file that answered. |\n\
             | `debug_id` | VARCHAR | Normalized cache key that answered. |\n\
             | `status` | VARCHAR | 'ok' / 'not_found' / 'no_line' / 'error:<kind>'. |"
                .into(),
        ));
        FunctionMetadata {
            description: "Resolve one (build_id, address) frame to a STRUCT with the inline chain \
                          collapsed into a list"
                .into(),
            examples: vec![FunctionExample {
                sql: "SELECT symbols.main.symbolicate('e4c1f2b9', 303600) AS frame;".into(),
                description: "Resolve one module-relative frame (build-id 'e4c1f2b9', address \
                              303600) to a STRUCT of function/file/line plus the collapsed inline \
                              chain; the STRUCT's `status` is 'not_found' until a matching symbol \
                              source is registered with `add_source`."
                    .into(),
                expected_output: None,
            }],
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        vec![
            ArgSpec::column(
                "build_id",
                0,
                "varchar",
                "The module's normalized debug-id, or its raw per-format build-id hex. The worker \
                 normalizes either form.",
            ),
            ArgSpec::column(
                "address",
                1,
                "uint64",
                "The MODULE-RELATIVE virtual address (before ASLR slide / load bias). The caller \
                 subtracts the module base first (the address-model contract).",
            ),
        ]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse::result(symbolicate_struct_type()))
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<RecordBatch> {
        let build = batch.column(0);
        let addr = batch.column(1);
        let rows = batch.num_rows();

        let mut functions: Vec<Option<String>> = Vec::with_capacity(rows);
        let mut files: Vec<Option<String>> = Vec::with_capacity(rows);
        let mut lines: Vec<Option<i32>> = Vec::with_capacity(rows);
        let mut modules: Vec<Option<String>> = Vec::with_capacity(rows);
        let mut debug_ids: Vec<Option<String>> = Vec::with_capacity(rows);
        let mut statuses: Vec<String> = Vec::with_capacity(rows);
        #[allow(clippy::type_complexity)]
        let mut inline_chains: Vec<Vec<(Option<String>, Option<String>, Option<i32>)>> =
            Vec::with_capacity(rows);

        with_state(|state| {
            for i in 0..rows {
                let frames = match (build_id_at(build, i), address_at(addr, i)) {
                    (Some(b), Some(a)) => state.resolve(&b, a),
                    _ => vec![ResolvedFrame::not_found()],
                };
                // Physical frame = last (highest inline_depth); inline frames precede it.
                let physical = frames
                    .last()
                    .cloned()
                    .unwrap_or_else(ResolvedFrame::not_found);
                functions.push(physical.function.clone());
                files.push(physical.file.clone());
                lines.push(physical.line.map(|l| l as i32));
                modules.push(physical.module.clone());
                debug_ids.push(physical.debug_id.clone());
                statuses.push(physical.status.as_status());
                let chain: Vec<(Option<String>, Option<String>, Option<i32>)> = frames
                    [..frames.len().saturating_sub(1)]
                    .iter()
                    .map(|f| (f.function.clone(), f.file.clone(), f.line.map(|l| l as i32)))
                    .collect();
                inline_chains.push(chain);
            }
        });

        let function_arr: ArrayRef = Arc::new(functions.into_iter().collect::<StringArray>());
        let file_arr: ArrayRef = Arc::new(files.into_iter().collect::<StringArray>());
        let line_arr: ArrayRef = Arc::new(lines.into_iter().collect::<Int32Array>());
        let inlined_arr: ArrayRef = build_inline_list(&inline_chains);
        let module_arr: ArrayRef = Arc::new(modules.into_iter().collect::<StringArray>());
        let debug_id_arr: ArrayRef = Arc::new(debug_ids.into_iter().collect::<StringArray>());
        let status_arr: ArrayRef = Arc::new(StringArray::from(statuses));

        let sa = struct_array(vec![
            (Field::new("function", DataType::Utf8, true), function_arr),
            (Field::new("file", DataType::Utf8, true), file_arr),
            (Field::new("line", DataType::Int32, true), line_arr),
            (
                Field::new("inlined", inline_frames_list_type(), true),
                inlined_arr,
            ),
            (Field::new("module", DataType::Utf8, true), module_arr),
            (Field::new("debug_id", DataType::Utf8, true), debug_id_arr),
            (Field::new("status", DataType::Utf8, false), status_arr),
        ]);

        RecordBatch::try_new(params.output_schema.clone(), vec![Arc::new(sa) as ArrayRef])
            .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

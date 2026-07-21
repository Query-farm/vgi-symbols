//! `inline_frames(build_id, address) -> LIST<STRUCT(function, file, line)>` —
//! just the inline chain (innermost-first), without the physical frame.

use arrow_array::{ArrayRef, RecordBatch};
use symbols_core::frame::ResolvedFrame;
use vgi::{ArgSpec, BindParams, BindResponse, FunctionMetadata, ProcessParams, ScalarFunction};
use vgi_rpc::{Result, RpcError};

use crate::schema::{build_inline_list, inline_frames_list_type};
use crate::state::with_state;
use crate::util::{address_at, build_id_at};

/// `inline_frames`.
pub struct InlineFrames;

impl ScalarFunction for InlineFrames {
    fn name(&self) -> &str {
        "inline_frames"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Inlined Call Chain at an Address",
            "Resolve a `(build_id, address)` frame to just its inlined call chain — a \
             `LIST(STRUCT(function, file, line))` ordered innermost-first, **excluding** the \
             physical frame. Empty list when the address has no inlining (or no symbols). Useful \
             to reconstruct a logically-deeper stack than the captured frame count. `build_id` is \
             the normalized debug-id or raw build-id hex; `address` is the MODULE-RELATIVE virtual \
             address (caller subtracts the load base). Backed by the persistent build-id-keyed \
             cache.",
            "Resolve `(build_id, address)` to its inline chain only: `LIST(STRUCT(function, file, \
             line))`, innermost-first, without the physical frame. `address` is module-relative.",
            "inline frames, inlining, inline chain, symbolicate, dwarf, pdb, stack, build_id, \
             address",
        );
        tags.push(("vgi.category".into(), "Resolution".into()));
        // Described example carried as `vgi.example_queries` (the native examples
        // carrier drops descriptions → VGI515).
        tags.push((
            "vgi.example_queries".into(),
            r#"[{"description":"Get the length of the inlined call chain (innermost-first, physical frame excluded) at module-relative address 303600 in build-id 'e4c1f2b9'; 0 until the address has both symbols and inlining.","sql":"SELECT length(symbols.main.inline_frames('e4c1f2b9', 303600)) AS inline_depth"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "Resolve a (build_id, address) frame to its inline chain only (a list of \
                          STRUCT(function, file, line))"
                .into(),
            return_type: Some(inline_frames_list_type()),
            examples: Vec::new(),
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
                "The module's normalized debug-id, or its raw per-format build-id hex.",
            ),
            ArgSpec::column(
                "address",
                1,
                "uint64",
                "The MODULE-RELATIVE virtual address (before ASLR slide / load bias).",
            ),
        ]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse::result(inline_frames_list_type()))
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<RecordBatch> {
        let build = batch.column(0);
        let addr = batch.column(1);
        let rows = batch.num_rows();
        #[allow(clippy::type_complexity)]
        let mut chains: Vec<Vec<(Option<String>, Option<String>, Option<i32>)>> =
            Vec::with_capacity(rows);
        with_state(|state| {
            for i in 0..rows {
                let frames = match (build_id_at(build, i), address_at(addr, i)) {
                    (Some(b), Some(a)) => state.resolve(&b, a),
                    _ => vec![ResolvedFrame::not_found()],
                };
                let chain = frames[..frames.len().saturating_sub(1)]
                    .iter()
                    .map(|f| (f.function.clone(), f.file.clone(), f.line.map(|l| l as i32)))
                    .collect();
                chains.push(chain);
            }
        });
        let out: ArrayRef = build_inline_list(&chains);
        RecordBatch::try_new(params.output_schema.clone(), vec![out])
            .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

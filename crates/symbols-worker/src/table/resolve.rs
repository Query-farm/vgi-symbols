//! `resolve(build_id, address) -> TABLE(...)` — the LATERAL workhorse. Emits the
//! inline-expanded rows (one row per input frame × inline level, innermost-first
//! with the physical frame last), echoing `build_id`/`address` so the caller
//! joins back to the source frame. This is the JOIN surface for a `stack_frames`
//! table; resolution never drops a frame and never errors the scan.

use arrow_array::RecordBatch;
use vgi::table_in_out::TableInOutFunction;
use vgi::{ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams};
use vgi_rpc::Result;

use crate::schema::{resolved_schema, ResolvedBatchBuilder, ResolvedRows};
use crate::state::with_state;
use crate::util::{address_at, build_id_at};

/// `resolve`.
pub struct Resolve;

impl TableInOutFunction for Resolve {
    fn name(&self) -> &str {
        "resolve"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Resolve Frames (LATERAL)",
            "The bulk symbolication JOIN: resolve a column of `(build_id, address)` stack frames to \
             their inline-expanded source frames. Used LATERAL against a `stack_frames` table, it \
             emits one row per (input frame × inline level), ordered innermost-first with the \
             physical frame last (highest `inline_depth`, `is_inline=false`); `build_id` and \
             `address` are echoed so you can join back. An address with no matching module yields a \
             single `status='not_found'` row (NULL symbols) — resolution never drops a frame and \
             never errors the scan, so the JOIN stays total. `address` is the MODULE-RELATIVE \
             virtual address (the caller subtracts the load base). Backed by the persistent, \
             build-id-keyed debug-info cache, so a column of millions of frames parses each module \
             exactly once.",
            "Resolve a column of `(build_id, address)` frames to inline-expanded rows: \
             `FROM frames f, LATERAL symbols.resolve(f.build_id, f.address) r`. One row per inline \
             level, innermost-first, physical frame last. `address` is module-relative.",
            "resolve, symbolicate, lateral, join, stack frames, inline expansion, addr2line, \
             dwarf, pdb, profiling, minidump, crash, build_id, address",
        );
        tags.push(("vgi.result_columns_md".into(), RESULT_COLUMNS_MD.into()));
        FunctionMetadata {
            description: "Resolve a column of (build_id, address) frames to inline-expanded \
                          function/file/line rows (LATERAL)"
                .into(),
            examples: vec![FunctionExample {
                sql: "SELECT f.frame_idx, r.inline_depth, r.function, r.file, r.line \
                      FROM stack_frames f, LATERAL symbols.resolve(f.build_id, f.address) r \
                      ORDER BY f.frame_idx, r.inline_depth;"
                    .into(),
                description: "Bulk-symbolicate a stack-frame table as a JOIN.".into(),
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
                "The module's normalized debug-id, or its raw per-format build-id hex (ELF GNU \
                 build-id / Mach-O UUID / PDB GUID). The worker normalizes either form.",
            ),
            ArgSpec::column(
                "address",
                1,
                "uint64",
                "The MODULE-RELATIVE virtual address (within the module image, before ASLR slide / \
                 load bias). The caller — or the upstream minidump/pprof/perf extractor — subtracts \
                 the module base first.",
            ),
        ]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse {
            output_schema: resolved_schema(false),
            opaque_data: Vec::new(),
        })
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<Vec<RecordBatch>> {
        // `resolve` transforms an input batch of (build_id, address) columns into
        // inline-expanded rows. With no input rows there is nothing to do —
        // return an empty batch (NOT a row per empty tick, which would loop the
        // exchange). The current DuckDB+vgi binding only allows literal
        // table-function params, so for the bulk JOIN use the scalar
        // `symbolicate` over a frame column, or `resolve_batch` over a LIST.
        if batch.num_columns() < 2 || batch.num_rows() == 0 {
            return Ok(vec![ResolvedBatchBuilder::new(false).finish()?]);
        }
        let build = batch.column(0);
        let addr = batch.column(1);
        let rows = batch.num_rows();
        let mut builder = ResolvedBatchBuilder::new(false);
        with_state(|state| {
            for i in 0..rows {
                let (b, a) = (build_id_at(build, i), address_at(addr, i));
                let frames = match (b.as_deref(), a) {
                    (Some(b), Some(a)) => state.resolve(b, a),
                    _ => vec![symbols_core::frame::ResolvedFrame::not_found()],
                };
                builder.push(&ResolvedRows {
                    frame_idx: None,
                    build_id: b.unwrap_or_default(),
                    address: a.unwrap_or(0),
                    frames,
                });
            }
        });
        let out = builder.finish()?;
        // Narrow to the (possibly projection-pruned) output schema by name.
        Ok(vec![vgi::table_in_out::project_batch(
            &out,
            &params.output_schema,
        )?])
    }
}

const RESULT_COLUMNS_MD: &str = "One row per (input frame × inline level), innermost-first:\n\n\
| column | type | description |\n\
|---|---|---|\n\
| `build_id` | VARCHAR | Echoed input build-id. |\n\
| `address` | UBIGINT | Echoed input address. |\n\
| `inline_depth` | INTEGER | 0 = innermost inlined function … N = the physical frame. |\n\
| `is_inline` | BOOLEAN | True for synthesized inline frames, false for the physical frame. |\n\
| `function` | VARCHAR | Demangled function name. |\n\
| `function_raw` | VARCHAR | The mangled / linkage name as stored. |\n\
| `file` | VARCHAR | Source file path. |\n\
| `line` | INTEGER | 1-based source line. |\n\
| `column` | INTEGER | Source column if present, else NULL. |\n\
| `module` | VARCHAR | Debug file that answered. |\n\
| `debug_id` | VARCHAR | Normalized cache key that answered. |\n\
| `status` | VARCHAR | 'ok' / 'not_found' / 'no_line' / 'error:<kind>'. |";

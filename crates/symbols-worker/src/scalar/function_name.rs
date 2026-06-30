//! `function_name(build_id, address) -> VARCHAR` — the innermost function name
//! only (the fast path for "just give me a label" — a flamegraph leaf or a
//! GROUP BY key), skipping file/line/inline materialization.

use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use arrow_schema::DataType;
use vgi::{
    ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams,
    ScalarFunction,
};
use vgi_rpc::{Result, RpcError};

use crate::state::with_state;
use crate::util::{address_at, build_id_at};

/// `function_name`.
pub struct FunctionName;

impl ScalarFunction for FunctionName {
    fn name(&self) -> &str {
        "function_name"
    }

    fn metadata(&self) -> FunctionMetadata {
        let tags = crate::meta::object_tags(
            "Resolve Function Name",
            "Resolve a `(build_id, address)` frame to just the innermost function name (demangled), \
             or NULL if no symbols are found. The cheap path that skips file/line and inline-chain \
             materialization — ideal as a flamegraph leaf label or a GROUP BY key for \
             crash-bucketing across a fleet. `build_id` is the module's normalized debug-id or its \
             raw per-format build-id hex (the worker normalizes); `address` is the \
             **module-relative** virtual address (the caller subtracts the load base — see the \
             address-model contract). Backed by the persistent build-id-keyed debug-info cache, so \
             a column of millions of addresses parses each module once.",
            "Resolve `(build_id, address)` to the innermost demangled function name (NULL if not \
             found). `address` is module-relative (caller subtracts the load base). Cheaper than \
             `resolve`/`symbolicate` — no file/line/inline.",
            "function name, symbolicate, resolve, addr2line, flamegraph leaf, group by, crash \
             bucket, build_id, address",
        );
        FunctionMetadata {
            description: "Resolve a (build_id, address) frame to the innermost function name only"
                .into(),
            return_type: Some(DataType::Utf8),
            examples: vec![FunctionExample {
                sql: "SELECT symbols.main.function_name('e4c1f2b9', 0x4a1f0);".into(),
                description: "Resolve a frame to its crash-site function name.".into(),
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
                "The MODULE-RELATIVE virtual address (the address within the module image, before \
                 ASLR slide / load bias). The caller — or the upstream minidump/pprof/perf \
                 extractor — subtracts the module base first.",
            ),
        ]
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse::result(DataType::Utf8))
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<RecordBatch> {
        let build = batch.column(0);
        let addr = batch.column(1);
        let rows = batch.num_rows();
        let mut out = Vec::with_capacity(rows);
        with_state(|state| {
            for i in 0..rows {
                let name = match (build_id_at(build, i), address_at(addr, i)) {
                    (Some(b), Some(a)) => state.function_name(&b, a),
                    _ => None,
                };
                out.push(name);
            }
        });
        let arr: StringArray = out.into_iter().collect();
        RecordBatch::try_new(
            params.output_schema.clone(),
            vec![Arc::new(arr) as ArrayRef],
        )
        .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

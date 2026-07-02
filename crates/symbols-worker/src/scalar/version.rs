//! `symbols_version()` — the worker's version string.

use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, StringArray};
use arrow_schema::DataType;
use vgi::{
    ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams,
    ScalarFunction,
};
use vgi_rpc::{Result, RpcError};

/// `symbols_version()`.
pub struct SymbolsVersion;

impl ScalarFunction for SymbolsVersion {
    fn name(&self) -> &str {
        "symbols_version"
    }

    fn metadata(&self) -> FunctionMetadata {
        let mut tags = crate::meta::object_tags(
            "Symbols Worker Version",
            "Return the version string of the running vgi-symbols worker binary (the worker's own \
             build version, a semver MAJOR.MINOR.PATCH, not the SDK/protocol version). Takes no \
             arguments and is deterministic — always the same single VARCHAR (never NULL) for a \
             given build. Useful for diagnostics and confirming which build is attached.",
            "Return the vgi-symbols worker version string, e.g. `symbols_version()` -> '0.1.0'. \
             Argument-free and deterministic.",
            "version, build version, symbols_version, diagnostics, worker version, semver",
        );
        tags.push(("vgi.category".into(), "Diagnostics".into()));
        tags.push((
            "vgi.executable_examples".into(),
            r#"[{"description":"Return the worker version string.","sql":"SELECT symbols.main.symbols_version() AS version"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: "Returns the vgi-symbols worker version string".into(),
            return_type: Some(DataType::Utf8),
            examples: vec![FunctionExample {
                sql: "SELECT symbols.main.symbols_version();".into(),
                description: "Return the vgi-symbols worker version string.".into(),
                expected_output: None,
            }],
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        Vec::new()
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse::result(DataType::Utf8))
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<RecordBatch> {
        let rows = batch.num_rows();
        let out: ArrayRef = Arc::new(StringArray::from(vec![symbols_core::version(); rows]));
        RecordBatch::try_new(params.output_schema.clone(), vec![out])
            .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

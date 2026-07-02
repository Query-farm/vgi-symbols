//! `demangle(mangled [, lang]) -> VARCHAR` — Itanium C++ / Rust (legacy + v0) /
//! MSVC / Swift demangling. Pure: no cache, no module needed.

use arrow_array::cast::AsArray;
use arrow_array::{Array, ArrayRef, RecordBatch, StringArray};
use arrow_schema::DataType;
use symbols_core::demangle::{demangle, DemangleLang};
use vgi::{
    ArgSpec, BindParams, BindResponse, FunctionExample, FunctionMetadata, ProcessParams,
    ScalarFunction,
};
use vgi_rpc::{Result, RpcError};

/// `demangle`. DuckDB scalars take only positional args, so the optional `lang`
/// is a 2nd positional const registered as a separate arity overload.
pub struct Demangle {
    /// Whether this overload accepts the positional `lang` argument.
    pub with_lang: bool,
}

impl ScalarFunction for Demangle {
    fn name(&self) -> &str {
        "demangle"
    }

    fn metadata(&self) -> FunctionMetadata {
        let description = if self.with_lang {
            "Demangle a raw linkage/symbol name into a readable function name using the given \
             mangling scheme (auto/cpp/rust/msvc/swift)"
        } else {
            "Demangle a raw linkage/symbol name into a readable function name (scheme auto-detected)"
        };
        let mut tags = crate::meta::object_tags(
            if self.with_lang {
                "Demangle Symbol (with language)"
            } else {
                "Demangle Symbol"
            },
            "Turn a mangled linkage name into a human-readable function name. Supports Itanium C++ \
             (`_Z...`), Rust legacy (`_ZN...`) and v0 (`_R...`), Microsoft Visual C++ (`?...`), and \
             Swift mangling. The optional second argument `lang` selects the scheme — 'auto' (the \
             default, detect from the string), 'cpp', 'rust', 'msvc', or 'swift'. Pure and total: \
             an unmangled or unrecognized name is returned unchanged, never an error. Use it when \
             you already have raw names from another tool and want them readable in SQL.",
            "Demangle a mangled symbol, e.g. `demangle('_ZN3foo3barEv')` -> 'foo::bar'. Optional \
             positional `lang`: auto (default) / cpp / rust / msvc / swift. Returns the input \
             unchanged if it is not mangled.",
            "demangle, mangled, symbol, linkage name, c++, itanium, rust, msvc, swift, function \
             name",
        );
        tags.push(("vgi.category".into(), "Demangling".into()));
        tags.push((
            "vgi.executable_examples".into(),
            r#"[{"description":"Demangle an Itanium C++ symbol.","sql":"SELECT symbols.main.demangle('_ZN3foo3barEv') AS name"}]"#
                .into(),
        ));
        FunctionMetadata {
            description: description.into(),
            return_type: Some(DataType::Utf8),
            examples: vec![FunctionExample {
                sql: "SELECT symbols.main.demangle('_ZN3foo3barEv');".into(),
                description: "Demangle an Itanium C++ symbol to 'foo::bar'.".into(),
                expected_output: None,
            }],
            tags,
            ..Default::default()
        }
    }

    fn argument_specs(&self) -> Vec<ArgSpec> {
        let mut specs = vec![ArgSpec::any_column(
            "mangled",
            0,
            "The raw mangled or linkage symbol name to demangle; a name that is not mangled is \
             returned unchanged.",
        )];
        if self.with_lang {
            specs.push(ArgSpec::const_arg(
                "lang",
                1,
                "varchar",
                "Mangling scheme: 'auto' (default — detect from the string), 'cpp', 'rust', \
                 'msvc', or 'swift'.",
            ));
        }
        specs
    }

    fn on_bind(&self, _params: &BindParams) -> Result<BindResponse> {
        Ok(BindResponse::result(DataType::Utf8))
    }

    fn process(&self, params: &ProcessParams, batch: &RecordBatch) -> Result<RecordBatch> {
        let lang = if self.with_lang {
            let s = params
                .arguments
                .const_str(1)
                .unwrap_or_else(|| "auto".into());
            DemangleLang::parse(&s).map_err(RpcError::value_error)?
        } else {
            DemangleLang::Auto
        };
        let col = batch.column(0);
        let names = string_col(col)?;
        let out: StringArray = (0..batch.num_rows())
            .map(|i| {
                if names.is_null(i) {
                    None
                } else {
                    Some(demangle(names.value(i), lang))
                }
            })
            .collect();
        RecordBatch::try_new(
            params.output_schema.clone(),
            vec![Arc::new(out) as ArrayRef],
        )
        .map_err(|e| RpcError::runtime_error(e.to_string()))
    }
}

use std::sync::Arc;

fn string_col(col: &ArrayRef) -> Result<arrow_array::StringArray> {
    match col.data_type() {
        DataType::Utf8 => Ok(col.as_string::<i32>().clone()),
        other => Err(RpcError::type_error(format!(
            "demangle: mangled must be VARCHAR, got {other:?}"
        ))),
    }
}

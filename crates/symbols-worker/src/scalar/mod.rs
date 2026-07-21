//! Scalar functions exposed by the symbols worker.

mod demangle;
mod function_name;
mod inline_frames;
mod symbolicate;

use vgi::Worker;

/// Register every scalar function on the worker.
pub fn register(worker: &mut Worker) {
    worker.register_scalar(symbolicate::Symbolicate);
    worker.register_scalar(function_name::FunctionName);
    worker.register_scalar(inline_frames::InlineFrames);
    // Two arity overloads: (mangled) and (mangled, lang).
    worker.register_scalar(demangle::Demangle { with_lang: false });
    worker.register_scalar(demangle::Demangle { with_lang: true });
}

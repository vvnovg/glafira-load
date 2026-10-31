//! Заполняется в Task 7.
use crate::{ContractInput, ParseError, ParseOptions, ParseResult};

pub fn parse(input: &ContractInput, _opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let mut out = ParseResult::default();
    out.warn(input.path, None, "importer not implemented yet");
    Ok(out)
}

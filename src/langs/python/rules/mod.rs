mod docstring_line_length;
mod docstring_start;
mod docstring_style;
mod local_imports;
mod string_annotations;

use crate::engine::rule::Rule;

pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(local_imports::LocalImports),
        Box::new(string_annotations::StringAnnotations),
        Box::new(docstring_style::DocstringStyle),
        Box::new(docstring_start::DocstringStart),
        Box::new(docstring_line_length::DocstringLineLength),
    ]
}

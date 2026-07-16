mod casing;
mod const_final;
mod dict_call;
mod docstring_line_length;
mod docstring_start;
mod docstring_style;
mod local_imports;
mod no_emoji;
mod string_annotations;

use crate::engine::rule::Rule;

pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(local_imports::LocalImports),
        Box::new(string_annotations::StringAnnotations),
        Box::new(docstring_style::DocstringStyle),
        Box::new(docstring_start::DocstringStart),
        Box::new(docstring_line_length::DocstringLineLength),
        Box::new(dict_call::DictCall),
        Box::new(const_final::ConstFinal),
        Box::new(casing::CasingEnumKey),
        Box::new(casing::CasingEnumVal),
        Box::new(casing::CasingModuleConst),
        Box::new(no_emoji::NoEmoji),
    ]
}

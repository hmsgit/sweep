mod allowed_emojis;
mod annotate_module_const;
mod casing;
mod comments_no_echo;
mod dict_style;
mod docstring_line_length;
mod docstring_no_echo;
mod docstring_no_type_echo;
mod docstring_start;
mod docstring_style;
mod docstring_sync;
mod imports_ban_local;
mod no_emdash;
mod string_annotations;

use crate::engine::rule::Rule;

pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(imports_ban_local::ImportsBanLocal),
        Box::new(string_annotations::StringAnnotations),
        Box::new(docstring_style::DocstringStyle),
        Box::new(docstring_start::DocstringStart),
        Box::new(docstring_line_length::DocstringLineLength),
        Box::new(dict_style::DictStyle),
        Box::new(annotate_module_const::AnnotateModuleConst),
        Box::new(casing::CasingEnumKey),
        Box::new(casing::CasingEnumVal),
        Box::new(casing::CasingModuleConst),
        Box::new(allowed_emojis::AllowedEmojis),
        Box::new(no_emdash::NoEmdash),
        Box::new(comments_no_echo::CommentsNoEcho),
        Box::new(docstring_sync::DocstringSync),
        Box::new(docstring_no_echo::DocstringNoEcho),
        Box::new(docstring_no_type_echo::DocstringNoTypeEcho),
    ]
}

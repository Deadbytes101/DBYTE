use dbyte_ast::TypeAnnotation;

#[derive(Debug, Clone)]
pub enum StdlibExport {
    Function {
        params: Vec<TypeAnnotation>,
        ret: TypeAnnotation,
    },
}

pub fn stdlib_exports(module: &str) -> Option<Vec<(String, StdlibExport)>> {
    match module {
        "std.math" => Some(vec![
            (
                "abs".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Int],
                    ret: TypeAnnotation::Int,
                },
            ),
            (
                "min".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Int, TypeAnnotation::Int],
                    ret: TypeAnnotation::Int,
                },
            ),
            (
                "max".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Int, TypeAnnotation::Int],
                    ret: TypeAnnotation::Int,
                },
            ),
        ]),
        "std.fs" => Some(vec![
            (
                "read_text".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str],
                    ret: TypeAnnotation::Str,
                },
            ),
            (
                "write_text".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str, TypeAnnotation::Str],
                    ret: TypeAnnotation::Inferred,
                },
            ),
        ]),
        "std.env" => Some(vec![(
            "args".into(),
            StdlibExport::Function {
                params: vec![],
                ret: TypeAnnotation::List(Box::new(TypeAnnotation::Str)),
            },
        )]),
        _ => None,
    }
}

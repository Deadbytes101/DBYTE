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
            (
                "read_bytes".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str],
                    ret: TypeAnnotation::Bytes,
                },
            ),
            (
                "write_bytes".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str, TypeAnnotation::Bytes],
                    ret: TypeAnnotation::Inferred,
                },
            ),
        ]),
        "std.encoding" => Some(vec![
            (
                "hex_encode".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Bytes],
                    ret: TypeAnnotation::Str,
                },
            ),
            (
                "hex_decode".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str],
                    ret: TypeAnnotation::Bytes,
                },
            ),
        ]),
        "std.hash" => Some(vec![(
            "sha256".into(),
            StdlibExport::Function {
                params: vec![TypeAnnotation::Bytes],
                ret: TypeAnnotation::Bytes,
            },
        )]),
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

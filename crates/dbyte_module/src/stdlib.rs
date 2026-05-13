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
            (
                "exists".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str],
                    ret: TypeAnnotation::Int,
                },
            ),
            (
                "mkdir".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str],
                    ret: TypeAnnotation::Inferred,
                },
            ),
            (
                "remove".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str],
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
        "std.buffer" => Some(vec![
            (
                "new".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Int],
                    ret: TypeAnnotation::Buffer,
                },
            ),
            (
                "from_bytes".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Bytes],
                    ret: TypeAnnotation::Buffer,
                },
            ),
            (
                "to_bytes".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Buffer],
                    ret: TypeAnnotation::Bytes,
                },
            ),
            (
                "len".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Buffer],
                    ret: TypeAnnotation::Int,
                },
            ),
            (
                "get".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Buffer, TypeAnnotation::Int],
                    ret: TypeAnnotation::Int,
                },
            ),
            (
                "set".into(),
                StdlibExport::Function {
                    params: vec![
                        TypeAnnotation::Buffer,
                        TypeAnnotation::Int,
                        TypeAnnotation::Int,
                    ],
                    ret: TypeAnnotation::Inferred,
                },
            ),
            (
                "slice".into(),
                StdlibExport::Function {
                    params: vec![
                        TypeAnnotation::Buffer,
                        TypeAnnotation::Int,
                        TypeAnnotation::Int,
                    ],
                    ret: TypeAnnotation::Bytes,
                },
            ),
            (
                "load".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str],
                    ret: TypeAnnotation::Buffer,
                },
            ),
            (
                "save".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Str, TypeAnnotation::Buffer],
                    ret: TypeAnnotation::Inferred,
                },
            ),
            (
                "find".into(),
                StdlibExport::Function {
                    params: vec![TypeAnnotation::Buffer, TypeAnnotation::Bytes],
                    ret: TypeAnnotation::Int,
                },
            ),
            (
                "replace".into(),
                StdlibExport::Function {
                    params: vec![
                        TypeAnnotation::Buffer,
                        TypeAnnotation::Int,
                        TypeAnnotation::Bytes,
                    ],
                    ret: TypeAnnotation::Inferred,
                },
            ),
        ]),
        "std.binary" => {
            let mut exports = Vec::new();
            let reader_funcs = [
                "u8", "i8", "u16_le", "u16_be", "i16_le", "i16_be", "u32_le", "u32_be", "i32_le",
                "i32_be",
            ];
            for name in reader_funcs {
                exports.push((
                    name.into(),
                    StdlibExport::Function {
                        params: vec![TypeAnnotation::Bytes, TypeAnnotation::Int],
                        ret: TypeAnnotation::Int,
                    },
                ));
            }

            let writer_funcs = ["pack_u16_le", "pack_u16_be", "pack_u32_le", "pack_u32_be"];
            for name in writer_funcs {
                exports.push((
                    name.into(),
                    StdlibExport::Function {
                        params: vec![TypeAnnotation::Int],
                        ret: TypeAnnotation::Bytes,
                    },
                ));
            }

            let buffer_writer_funcs = [
                "write_u16_le",
                "write_u16_be",
                "write_u32_le",
                "write_u32_be",
            ];
            for name in buffer_writer_funcs {
                exports.push((
                    name.into(),
                    StdlibExport::Function {
                        params: vec![
                            TypeAnnotation::Buffer,
                            TypeAnnotation::Int,
                            TypeAnnotation::Int,
                        ],
                        ret: TypeAnnotation::Inferred,
                    },
                ));
            }
            Some(exports)
        }
        _ => None,
    }
}

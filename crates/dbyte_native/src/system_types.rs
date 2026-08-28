//! Deterministic freestanding type and aggregate-layout rules for
//! `i686-dbyte-none`.
//!
//! The compiler host must never decide target widths through Rust casts or
//! `size_of`.  Every size, alignment, signedness rule, and literal check lives
//! here and is therefore independent of the machine running `dbytec`.

use crate::diagnostics::{Diagnostic, DiagnosticCategory, NativeResult};
use dbyte_ast::Span;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const POINTER_SIZE: u32 = 4;
pub const POINTER_ALIGNMENT: u32 = 4;
pub const MAX_ARRAY_LENGTH: u32 = 1_048_576;
pub const MAX_AGGREGATE_SIZE: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SystemType {
    Bool,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Usize,
    Str,
    Pointer(Box<SystemType>),
    Array(Box<SystemType>, u32),
    Struct(String),
}

impl SystemType {
    pub const fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::U8
                | Self::I16
                | Self::U16
                | Self::I32
                | Self::U32
                | Self::I64
                | Self::U64
                | Self::Usize
        )
    }

    pub const fn is_signed_integer(&self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32 | Self::I64)
    }

    pub const fn is_scalar(&self) -> bool {
        matches!(
            self,
            Self::Bool
                | Self::I8
                | Self::U8
                | Self::I16
                | Self::U16
                | Self::I32
                | Self::U32
                | Self::I64
                | Self::U64
                | Self::Usize
                | Self::Str
                | Self::Pointer(_)
        )
    }

    pub const fn is_pointer(&self) -> bool {
        matches!(self, Self::Pointer(_))
    }

    pub const fn is_wide_integer(&self) -> bool {
        matches!(self, Self::I64 | Self::U64)
    }

    pub const fn bit_width(&self) -> Option<u8> {
        match self {
            Self::I8 | Self::U8 => Some(8),
            Self::I16 | Self::U16 => Some(16),
            Self::I32 | Self::U32 | Self::Usize => Some(32),
            Self::I64 | Self::U64 => Some(64),
            _ => None,
        }
    }

    pub const fn scalar_size(&self) -> Option<u32> {
        match self {
            Self::Bool | Self::I8 | Self::U8 => Some(1),
            Self::I16 | Self::U16 => Some(2),
            Self::I32 | Self::U32 | Self::Usize | Self::Str | Self::Pointer(_) => Some(4),
            Self::I64 | Self::U64 => Some(8),
            Self::Array(_, _) | Self::Struct(_) => None,
        }
    }

    pub const fn scalar_alignment(&self) -> Option<u32> {
        match self {
            Self::I64 | Self::U64 => Some(4),
            _ => self.scalar_size(),
        }
    }
}

impl fmt::Display for SystemType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => formatter.write_str("bool"),
            Self::I8 => formatter.write_str("i8"),
            Self::U8 => formatter.write_str("u8"),
            Self::I16 => formatter.write_str("i16"),
            Self::U16 => formatter.write_str("u16"),
            Self::I32 => formatter.write_str("i32"),
            Self::U32 => formatter.write_str("u32"),
            Self::I64 => formatter.write_str("i64"),
            Self::U64 => formatter.write_str("u64"),
            Self::Usize => formatter.write_str("usize"),
            Self::Str => formatter.write_str("str"),
            Self::Pointer(inner) => write!(formatter, "ptr[{inner}]"),
            Self::Array(inner, length) => write!(formatter, "array[{inner}, {length}]"),
            Self::Struct(name) => formatter.write_str(name),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeLayout {
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructFieldDecl {
    pub name: String,
    pub ty: SystemType,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub packed: bool,
    pub fields: Vec<StructFieldDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: String,
    pub ty: SystemType,
    pub offset: u32,
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructLayout {
    pub name: String,
    pub packed: bool,
    pub fields: Vec<FieldLayout>,
    pub size: u32,
    pub alignment: u32,
}

impl StructLayout {
    pub fn field(&self, name: &str) -> Option<&FieldLayout> {
        self.fields.iter().find(|field| field.name == name)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeLayouts {
    structs: BTreeMap<String, StructLayout>,
}

impl TypeLayouts {
    pub fn build(declarations: &[StructDecl]) -> NativeResult<Self> {
        let mut declaration_map = BTreeMap::new();
        for declaration in declarations {
            if declaration_map
                .insert(declaration.name.clone(), declaration)
                .is_some()
            {
                return Err(Diagnostic::new(
                    DiagnosticCategory::DuplicateType,
                    format!("duplicate native struct '{}'", declaration.name),
                    declaration.span,
                ));
            }
        }

        let mut layouts = Self::default();
        let mut visiting = BTreeSet::new();
        for name in declaration_map.keys() {
            layouts.build_one(name, &declaration_map, &mut visiting)?;
        }
        Ok(layouts)
    }

    pub fn structure(&self, name: &str) -> Option<&StructLayout> {
        self.structs.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &StructLayout)> {
        self.structs
            .iter()
            .map(|(name, layout)| (name.as_str(), layout))
    }

    pub fn layout_of(&self, ty: &SystemType, span: Span) -> NativeResult<TypeLayout> {
        if let (Some(size), Some(alignment)) = (ty.scalar_size(), ty.scalar_alignment()) {
            return Ok(TypeLayout { size, alignment });
        }
        match ty {
            SystemType::Array(element, length) => {
                if *length == 0 || *length > MAX_ARRAY_LENGTH {
                    return Err(Diagnostic::new(
                        DiagnosticCategory::BadArrayLength,
                        format!("native array length {length} is outside 1..={MAX_ARRAY_LENGTH}"),
                        span,
                    ));
                }
                let element = self.layout_of(element, span)?;
                let size = element.size.checked_mul(*length).ok_or_else(|| {
                    Diagnostic::new(
                        DiagnosticCategory::LayoutOverflow,
                        "native array size overflow",
                        span,
                    )
                })?;
                ensure_aggregate_bound(size, span)?;
                Ok(TypeLayout {
                    size,
                    alignment: element.alignment,
                })
            }
            SystemType::Struct(name) => {
                let layout = self.structs.get(name).ok_or_else(|| {
                    Diagnostic::new(
                        DiagnosticCategory::UnknownType,
                        format!("unknown native struct '{name}'"),
                        span,
                    )
                })?;
                Ok(TypeLayout {
                    size: layout.size,
                    alignment: layout.alignment,
                })
            }
            _ => Err(Diagnostic::new(
                DiagnosticCategory::InvalidType,
                format!("native type '{ty}' has no object layout"),
                span,
            )),
        }
    }

    fn build_one(
        &mut self,
        name: &str,
        declarations: &BTreeMap<String, &StructDecl>,
        visiting: &mut BTreeSet<String>,
    ) -> NativeResult<()> {
        if self.structs.contains_key(name) {
            return Ok(());
        }
        let declaration = declarations.get(name).copied().ok_or_else(|| {
            Diagnostic::without_span(
                DiagnosticCategory::UnknownType,
                format!("unknown native struct '{name}'"),
            )
        })?;
        if !visiting.insert(name.to_owned()) {
            return Err(Diagnostic::new(
                DiagnosticCategory::InvalidType,
                format!("recursive by-value native struct '{name}' has no finite layout"),
                declaration.span,
            ));
        }

        let mut seen_fields = BTreeSet::new();
        let mut offset = 0u32;
        let mut struct_alignment = 1u32;
        let mut fields = Vec::with_capacity(declaration.fields.len());
        for field in &declaration.fields {
            if !seen_fields.insert(field.name.as_str()) {
                return Err(Diagnostic::new(
                    DiagnosticCategory::DuplicateField,
                    format!(
                        "duplicate field '{}' in native struct '{}'",
                        field.name, declaration.name
                    ),
                    field.span,
                ));
            }
            self.ensure_nested_layout(&field.ty, declarations, visiting, field.span)?;
            let field_layout = self.layout_of(&field.ty, field.span)?;
            let field_alignment = if declaration.packed {
                1
            } else {
                field_layout.alignment
            };
            offset = align_up(offset, field_alignment, field.span)?;
            let field_offset = offset;
            offset = offset.checked_add(field_layout.size).ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticCategory::LayoutOverflow,
                    format!("native struct '{}' size overflow", declaration.name),
                    field.span,
                )
            })?;
            struct_alignment = struct_alignment.max(field_alignment);
            fields.push(FieldLayout {
                name: field.name.clone(),
                ty: field.ty.clone(),
                offset: field_offset,
                size: field_layout.size,
                alignment: field_alignment,
            });
        }
        let size = align_up(offset, struct_alignment, declaration.span)?;
        ensure_aggregate_bound(size, declaration.span)?;
        visiting.remove(name);
        self.structs.insert(
            name.to_owned(),
            StructLayout {
                name: name.to_owned(),
                packed: declaration.packed,
                fields,
                size,
                alignment: struct_alignment,
            },
        );
        Ok(())
    }

    fn ensure_nested_layout(
        &mut self,
        ty: &SystemType,
        declarations: &BTreeMap<String, &StructDecl>,
        visiting: &mut BTreeSet<String>,
        span: Span,
    ) -> NativeResult<()> {
        match ty {
            SystemType::Struct(name) => self.build_one(name, declarations, visiting),
            SystemType::Array(element, _) => {
                self.ensure_nested_layout(element, declarations, visiting, span)
            }
            // Pointers never require the pointee to have a completed layout.
            SystemType::Pointer(_) => Ok(()),
            _ if ty.is_scalar() => Ok(()),
            _ => Err(Diagnostic::new(
                DiagnosticCategory::InvalidType,
                format!("cannot lay out native type '{ty}'"),
                span,
            )),
        }
    }
}

pub fn checked_literal_bits(value: i64, ty: &SystemType, span: Span) -> NativeResult<u32> {
    let in_range = match ty {
        SystemType::I8 => i8::try_from(value).is_ok(),
        SystemType::U8 => u8::try_from(value).is_ok(),
        SystemType::I16 => i16::try_from(value).is_ok(),
        SystemType::U16 => u16::try_from(value).is_ok(),
        SystemType::I32 => i32::try_from(value).is_ok(),
        SystemType::U32 | SystemType::Usize => u32::try_from(value).is_ok(),
        _ => {
            return Err(Diagnostic::new(
                DiagnosticCategory::InvalidNarrowing,
                format!("integer literal cannot initialize native type '{ty}'"),
                span,
            ))
        }
    };
    if !in_range {
        return Err(Diagnostic::new(
            DiagnosticCategory::IntegerOutOfRange,
            format!("integer literal {value} is outside native type '{ty}'"),
            span,
        ));
    }
    Ok(value as u32)
}

pub fn explicit_cast_allowed(from: &SystemType, to: &SystemType) -> bool {
    (from.is_integer() && to.is_integer())
        || (matches!(from, SystemType::Usize) && to.is_pointer())
        || (from.is_pointer() && matches!(to, SystemType::Usize))
        || (matches!(from, SystemType::Usize) && matches!(to, SystemType::Str))
        || (matches!(from, SystemType::Str) && matches!(to, SystemType::Usize))
        || from == to
}

fn align_up(value: u32, alignment: u32, span: Span) -> NativeResult<u32> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(Diagnostic::new(
            DiagnosticCategory::LayoutOverflow,
            format!("invalid native alignment {alignment}"),
            span,
        ));
    }
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::LayoutOverflow,
                "native alignment arithmetic overflow",
                span,
            )
        })
}

fn ensure_aggregate_bound(size: u32, span: Span) -> NativeResult<()> {
    if size > MAX_AGGREGATE_SIZE {
        Err(Diagnostic::new(
            DiagnosticCategory::LayoutOverflow,
            format!("native aggregate size {size} exceeds bound {MAX_AGGREGATE_SIZE}"),
            span,
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span::new(1, 1)
    }

    #[test]
    fn i686_scalar_sizes_alignments_and_signedness_are_explicit() {
        let cases = [
            (SystemType::Bool, 1, false),
            (SystemType::I8, 1, true),
            (SystemType::U8, 1, false),
            (SystemType::I16, 2, true),
            (SystemType::U16, 2, false),
            (SystemType::I32, 4, true),
            (SystemType::U32, 4, false),
            (SystemType::Usize, 4, false),
            (SystemType::Pointer(Box::new(SystemType::U8)), 4, false),
        ];
        for (ty, size, signed) in cases {
            assert_eq!(ty.scalar_size(), Some(size));
            assert_eq!(ty.scalar_alignment(), Some(size));
            assert_eq!(ty.is_signed_integer(), signed);
        }
    }

    #[test]
    fn normal_and_packed_struct_layouts_are_declaration_ordered() {
        let declarations = [
            StructDecl {
                name: "Normal".into(),
                packed: false,
                fields: vec![
                    StructFieldDecl {
                        name: "byte".into(),
                        ty: SystemType::U8,
                        span: span(),
                    },
                    StructFieldDecl {
                        name: "word".into(),
                        ty: SystemType::U32,
                        span: span(),
                    },
                ],
                span: span(),
            },
            StructDecl {
                name: "Packed".into(),
                packed: true,
                fields: vec![
                    StructFieldDecl {
                        name: "byte".into(),
                        ty: SystemType::U8,
                        span: span(),
                    },
                    StructFieldDecl {
                        name: "word".into(),
                        ty: SystemType::U32,
                        span: span(),
                    },
                ],
                span: span(),
            },
        ];
        let layouts = TypeLayouts::build(&declarations).unwrap();
        let normal = layouts.structure("Normal").unwrap();
        assert_eq!(
            (normal.fields[1].offset, normal.size, normal.alignment),
            (4, 8, 4)
        );
        let packed = layouts.structure("Packed").unwrap();
        assert_eq!(
            (packed.fields[1].offset, packed.size, packed.alignment),
            (1, 5, 1)
        );
    }

    #[test]
    fn arrays_are_bounded_and_use_element_alignment() {
        let layouts = TypeLayouts::default();
        let layout = layouts
            .layout_of(&SystemType::Array(Box::new(SystemType::U16), 7), span())
            .unwrap();
        assert_eq!(
            layout,
            TypeLayout {
                size: 14,
                alignment: 2
            }
        );
        let error = layouts
            .layout_of(&SystemType::Array(Box::new(SystemType::U8), 0), span())
            .unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::BadArrayLength);
    }

    #[test]
    fn literal_narrowing_is_checked_before_bits_are_emitted() {
        assert_eq!(
            checked_literal_bits(255, &SystemType::U8, span()).unwrap(),
            255
        );
        assert_eq!(
            checked_literal_bits(-128, &SystemType::I8, span()).unwrap(),
            0xffff_ff80
        );
        assert_eq!(
            checked_literal_bits(256, &SystemType::U8, span())
                .unwrap_err()
                .category,
            DiagnosticCategory::IntegerOutOfRange
        );
    }

    #[test]
    fn only_usize_has_explicit_integer_pointer_conversion() {
        let pointer = SystemType::Pointer(Box::new(SystemType::U8));
        assert!(explicit_cast_allowed(&SystemType::Usize, &pointer));
        assert!(explicit_cast_allowed(&pointer, &SystemType::Usize));
        assert!(!explicit_cast_allowed(&SystemType::U32, &pointer));
    }

    #[test]
    fn every_integer_literal_boundary_is_target_checked() {
        let accepted = [
            (-128, SystemType::I8),
            (127, SystemType::I8),
            (0, SystemType::U8),
            (255, SystemType::U8),
            (-32_768, SystemType::I16),
            (32_767, SystemType::I16),
            (65_535, SystemType::U16),
            (i32::MIN as i64, SystemType::I32),
            (i32::MAX as i64, SystemType::I32),
            (u32::MAX as i64, SystemType::U32),
            (u32::MAX as i64, SystemType::Usize),
        ];
        for (value, ty) in accepted {
            checked_literal_bits(value, &ty, span()).unwrap();
        }
        for (value, ty) in [
            (-129, SystemType::I8),
            (128, SystemType::I8),
            (-1, SystemType::U8),
            (256, SystemType::U8),
            (-32_769, SystemType::I16),
            (32_768, SystemType::I16),
            (65_536, SystemType::U16),
            (i32::MAX as i64 + 1, SystemType::I32),
            (-1, SystemType::U32),
            (u32::MAX as i64 + 1, SystemType::Usize),
        ] {
            assert_eq!(
                checked_literal_bits(value, &ty, span())
                    .unwrap_err()
                    .category,
                DiagnosticCategory::IntegerOutOfRange
            );
        }
    }

    #[test]
    fn duplicate_and_recursive_by_value_fields_are_rejected() {
        let duplicate = StructDecl {
            name: "Duplicate".into(),
            packed: false,
            fields: vec![
                StructFieldDecl {
                    name: "field".into(),
                    ty: SystemType::U8,
                    span: span(),
                },
                StructFieldDecl {
                    name: "field".into(),
                    ty: SystemType::U16,
                    span: span(),
                },
            ],
            span: span(),
        };
        assert_eq!(
            TypeLayouts::build(&[duplicate]).unwrap_err().category,
            DiagnosticCategory::DuplicateField
        );

        let recursive = StructDecl {
            name: "Recursive".into(),
            packed: false,
            fields: vec![StructFieldDecl {
                name: "next".into(),
                ty: SystemType::Struct("Recursive".into()),
                span: span(),
            }],
            span: span(),
        };
        assert_eq!(
            TypeLayouts::build(&[recursive]).unwrap_err().category,
            DiagnosticCategory::InvalidType
        );
    }

    #[test]
    fn recursive_pointer_has_finite_i686_layout() {
        let declaration = StructDecl {
            name: "Node".into(),
            packed: false,
            fields: vec![
                StructFieldDecl {
                    name: "value".into(),
                    ty: SystemType::U32,
                    span: span(),
                },
                StructFieldDecl {
                    name: "next".into(),
                    ty: SystemType::Pointer(Box::new(SystemType::Struct("Node".into()))),
                    span: span(),
                },
            ],
            span: span(),
        };
        let layouts = TypeLayouts::build(&[declaration]).unwrap();
        let node = layouts.structure("Node").unwrap();
        assert_eq!((node.size, node.alignment), (8, 4));
    }
}

from __future__ import annotations

import argparse
import struct
from pathlib import Path

MAGIC = b"DBYRT641"
ABI_VERSION = 1
HEADER_BYTES = 64
MAX_FILE_BYTES = 384 * 512


def xor_dwords(data: bytes) -> int:
    padded = data + bytes((-len(data)) & 3)
    value = 0
    for offset in range(0, len(padded), 4):
        value ^= struct.unpack_from("<I", padded, offset)[0]
    return value

def elf64_symbol_value(image: bytes, symbol_name: str) -> int:
    if len(image) < 64 or image[:4] != b"\x7fELF":
        raise RuntimeError("entry object is not ELF")
    if image[4] != 2 or image[5] != 1:
        raise RuntimeError("entry object must be little-endian ELF64")
    section_offset = struct.unpack_from("<Q", image, 40)[0]
    section_size = struct.unpack_from("<H", image, 58)[0]
    section_count = struct.unpack_from("<H", image, 60)[0]
    if section_size < 64 or section_count == 0:
        raise RuntimeError("entry object has no bounded section table")
    section_end = section_offset + section_size * section_count
    if section_end > len(image):
        raise RuntimeError("entry object section table is truncated")

    sections = []
    for index in range(section_count):
        offset = section_offset + index * section_size
        sections.append(struct.unpack_from("<IIQQQQIIQQ", image, offset))
    matches = []
    for section in sections:
        section_type = section[1]
        if section_type != 2:  # SHT_SYMTAB
            continue
        sym_offset, sym_size, strtab_index, entry_size = (
            section[4], section[5], section[6], section[9]
        )
        if entry_size < 24 or strtab_index >= len(sections):
            raise RuntimeError("entry object has malformed symbol metadata")
        if sym_size % entry_size:
            raise RuntimeError("entry object symbol table has a partial entry")
        strtab = sections[strtab_index]
        str_offset, str_size = strtab[4], strtab[5]
        if sym_offset + sym_size > len(image) or str_offset + str_size > len(image):
            raise RuntimeError("entry object symbol data is truncated")
        strings = image[str_offset:str_offset + str_size]
        for offset in range(sym_offset, sym_offset + sym_size, entry_size):
            name_offset, info, _, section_index, value, _ = struct.unpack_from(
                "<IBBHQQ", image, offset
            )
            if name_offset >= len(strings):
                raise RuntimeError("entry object symbol name is out of bounds")
            name_end = strings.find(b"\0", name_offset)
            if name_end < 0:
                raise RuntimeError("entry object symbol name is unterminated")
            name = strings[name_offset:name_end].decode("utf-8", errors="strict")
            if name == symbol_name and section_index != 0 and (info & 0x0f) == 2:
                if section_index >= len(sections):
                    raise RuntimeError("entry symbol section is out of bounds")
                target = sections[section_index]
                if target[1] != 1 or (target[2] & 0x4) == 0:
                    raise RuntimeError("entry symbol is not in executable PROGBITS")
                matches.append(value)
    if len(matches) != 1:
        raise RuntimeError(
            f"entry symbol {symbol_name!r} count was {len(matches)} instead of one"
        )
    return matches[0]


def pack(payload: bytes, payload_entry_offset: int = 0) -> bytes:
    if not payload:
        raise RuntimeError("empty DBYRT64 payload")
    payload = payload + bytes((-len(payload)) & 15)
    image_bytes = HEADER_BYTES + len(payload)
    if image_bytes > MAX_FILE_BYTES:
        raise RuntimeError("DBYRT64 image exceeds HOPFS1 per-file limit")
    if payload_entry_offset < 0 or payload_entry_offset >= len(payload):
        raise RuntimeError("DBYRT64 entry offset is outside the payload")
    entry_offset = HEADER_BYTES + payload_entry_offset
    if entry_offset & 3:
        raise RuntimeError("DBYRT64 entry offset must be four-byte aligned")
    header = struct.pack(
        "<8s7I28s",
        MAGIC,
        ABI_VERSION,
        HEADER_BYTES,
        image_bytes,
        entry_offset,
        len(payload),
        0,
        xor_dwords(payload),
        bytes(28),
    )
    if len(header) != HEADER_BYTES:
        raise RuntimeError("DBYRT64 header size mismatch")
    return header + payload

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--entry-object", type=Path)
    parser.add_argument("--entry-symbol")
    args = parser.parse_args()
    if (args.entry_object is None) != (args.entry_symbol is None):
        raise RuntimeError("--entry-object and --entry-symbol must be used together")
    payload_entry_offset = 0
    if args.entry_object is not None:
        payload_entry_offset = elf64_symbol_value(
            args.entry_object.read_bytes(), args.entry_symbol
        )
    image = pack(args.input.read_bytes(), payload_entry_offset)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(image)
    print(
        f"DBYRT64 PACK PASS bytes={len(image)} "
        f"payload={len(image)-HEADER_BYTES} entry={HEADER_BYTES + payload_entry_offset}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
